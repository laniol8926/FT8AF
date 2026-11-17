//! FT8 QSO auto-sequencer — a pure state machine ported (in spirit) from
//! `FT8TransmitSignal` + `FunctionOfTransmit`. It reacts to the decoded messages
//! of each RX slot and decides the message to send in the next TX slot, handling
//! both roles: calling CQ (initiator) and answering a CQ (answerer). On QSO
//! completion it emits a `QsoRecord` to log. No I/O lives here, so it's fully
//! unit-testable; the scheduler turns its decisions into encode+PTT+DB+events.
//!
//! **Deliberately not ported**: Android's no-reply-counting also exempts a
//! weak-signal slot (`!messages.get(0).isWeakSignal`) from counting against
//! the TX Watchdog. `isWeakSignal` is set by Android's decoder based on
//! which decode *pass* found the message (`isDeep`, its second/subtraction
//! pass for marginal signals) -- an architectural concept desktop's
//! single-pass-per-slot decoder has no equivalent of at all (confirmed: no
//! per-message pass/confidence flag exists on `DecodedMessage`). Inventing
//! a substitute threshold off `DecodedMessage.score` would be guessing at a
//! cutoff Android never used this way, so this exemption is left out
//! rather than faked -- flagged here for whoever next touches this file.

use serde::{Deserialize, Serialize};

use crate::db::QsoRecord;
use crate::dsp::DecodedMessage;
use crate::util;

/// The last message we transmitted — drives what reply we expect next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxStage {
    Idle,
    Cq,
    Grid,    // answered a CQ with our grid
    Report,  // sent a signal report
    RReport, // sent R + report
    Rr73,
    Bye73,
}

/// Snapshot of QSO/TX state for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QsoStatus {
    pub active: bool,
    pub stage: TxStage,
    pub target: Option<String>,
    pub tx_message: Option<String>,
    pub report_sent: Option<i32>,
    pub report_rcvd: Option<i32>,
}

/// What the engine wants the scheduler to do after processing a slot.
#[derive(Debug, Clone)]
pub enum QsoOutcome {
    /// A QSO finished and should be logged.
    Completed(QsoRecord),
}

#[derive(Debug, Clone)]
enum Content {
    Grid(String),
    Report(i32),
    RReport(i32),
    Rr73,
    Rrr,
    Bye73,
    Other,
}

pub struct QsoEngine {
    pub my_call: String,
    pub my_grid: String, // 4-char
    pub band: String,
    pub freq_mhz: String,
    /// Operating mode ("FT8"/"FT4"), logged on every completed QSO. Set by the
    /// engine (mirroring `band`/`freq_mhz`) from `Mode::display_name()`, kept
    /// as a plain `String` here (not `crate::mode::Mode`) so `qso.rs` doesn't
    /// need to depend on it for what's otherwise just a logged label.
    pub mode: String,
    pub active: bool,
    pub auto_return_to_cq: bool,
    /// Settings -> Transmission's "TX Watchdog" ("Stop After" N unanswered
    /// attempts) -- ported in spirit from Android's `GeneralVariables.
    /// noReplyLimit`/`shouldGiveUpTarget`. `0` = unlimited (Android's own
    /// default), matching Android's own semantics exactly even though
    /// desktop's stage model differs (see `should_give_up_target`'s doc
    /// comment for why this re-derives the signal natively rather than
    /// porting Android's numeric `functionOrder` arithmetic line for line).
    pub no_reply_limit: u32,
    /// Settings -> Transmission's "Max 73 Sends" -- ported in spirit from
    /// Android's `GeneralVariables.max73Sends`/`hasReachedMax73Sends`.
    /// `0` = Auto/off (Android's own default).
    pub max_73_sends: u32,
    /// Settings -> Transmission's "CQ Watchdog" -- how many unanswered CQ
    /// cycles (no message addressed to us at all, same granularity as
    /// `no_reply_limit`) before Call CQ gives itself up, same as pressing
    /// Stop. Genuinely new: neither this app nor the Android fork has ever
    /// had a give-up concept for a bare CQ with no target yet (confirmed by
    /// reading `FT8TransmitSignal.java` -- `noReplyLimit`/`shouldGiveUpTarget`
    /// only ever apply once a target is picked). Deliberately a SEPARATE
    /// counter/limit from `no_reply_limit` rather than reusing it: an
    /// operator may want a flaky mid-QSO partner dropped quickly while
    /// still letting a bare CQ call run far longer (or forever) waiting to
    /// be heard, and conflating the two would silently couple those two
    /// unrelated intents. `0` = unlimited -- the deliberate default, since
    /// this is new functionality with no prior behavior to preserve except
    /// "runs until stopped," which is also exactly what an unattended/
    /// "robot" CQ operation needs.
    pub cq_no_reply_limit: u32,

    stage: TxStage,
    target: Option<String>,
    grid_rcvd: Option<String>,
    report_sent: Option<i32>,
    report_rcvd: Option<i32>,
    tx_message: Option<String>,
    started_ms: i64,
    /// True once the courtesy "73" queued in `TxStage::Bye73` has actually been
    /// handed to the transmitter (via `notify_transmitted`). Guards `finish_qso`
    /// so a Bye73 that missed its slot (late TX window, rig busy, encode failure)
    /// is retransmitted next slot instead of being clobbered by the wrap-up.
    bye73_sent: bool,
    /// Consecutive slots, for the current target, where no reply was heard
    /// at all (we're in `Grid`/`Report`/`RReport` -- still calling, not yet
    /// logged). Reset whenever the target changes or any reply advances the
    /// stage. Drives `no_reply_limit` / the "TX Watchdog" setting.
    no_reply_count: u32,
    /// Consecutive slots, while calling bare CQ, with no message addressed
    /// to us at all. Reset on `start_cq`/`stop` (via `reset_qso`) and on any
    /// real reply to our CQ (same "advanced" check that resets
    /// `no_reply_count`). Drives `cq_no_reply_limit` / the "CQ Watchdog"
    /// setting.
    cq_no_reply_count: u32,
    /// How many times `RR73`/`73` (stages `Rr73`/`Bye73`) has actually been
    /// transmitted for the current target (set in `notify_transmitted`,
    /// reset on target change). Drives `max_73_sends`.
    final_ack_sends: u32,
    /// The most recently *cleanly completed* QSO, remembered briefly so a
    /// repeat from that station (they missed our final RR73/73) is
    /// answered as a continuation of the same QSO rather than ignored or
    /// logged a second time. Ported from Android's `lastLoggedCall`/
    /// `stoppedAfterQso`/`answerRepeatAfterLog`. Only ever set by
    /// `finish_qso` when `auto_return_to_cq` is off (Android's own
    /// "TX switched itself off after a logged QSO" precondition) --
    /// `give_up_target` (TX Watchdog / Max 73 Sends cap) never sets it, so
    /// a station the operator's own limit gave up on is structurally never
    /// eligible for a repeat answer, matching Android's "operator's limit
    /// wins" `cappedCallsign` check without needing to port that flag
    /// separately.
    dormant: Option<DormantQso>,
    /// Set while re-answering a `dormant` repeat, so the completion it
    /// eventually reaches suppresses the `QsoOutcome` (no duplicate log
    /// entry) instead of being gated by stage/content the way a normal
    /// completion is.
    answering_dormant_repeat: bool,
}

/// See `QsoEngine::dormant`'s doc comment.
#[derive(Debug, Clone)]
struct DormantQso {
    call: String,
    band: String,
    /// The ORIGINAL QSO's start time -- a repeat answer keeps this
    /// (Android's `keepsQsoStart`) rather than stamping a fresh one, so
    /// TIME_ON never changes if the courtesy 73 just needed a second try.
    started_ms: i64,
    ended_ms: i64,
    repeat_answers: u32,
}

/// How long after a clean completion a repeat from that same station still
/// counts as the same QSO. Matches Android's `SAME_QSO_WINDOW_MS`.
const SAME_QSO_WINDOW_MS: i64 = 5 * 60 * 1000;
/// Caps how many times a repeat is answered per completed QSO. Matches
/// Android's `MAX_REPEAT_ANSWERS`.
const MAX_REPEAT_ANSWERS: u32 = 2;

impl QsoEngine {
    pub fn new(my_call: impl Into<String>, my_grid: impl Into<String>) -> Self {
        QsoEngine {
            my_call: my_call.into().to_uppercase(),
            my_grid: util::grid4(&my_grid.into()).to_uppercase(),
            band: String::new(),
            freq_mhz: String::new(),
            mode: "FT8".to_string(),
            active: false,
            auto_return_to_cq: true,
            no_reply_limit: 0,
            max_73_sends: 0,
            cq_no_reply_limit: 0,
            stage: TxStage::Idle,
            target: None,
            grid_rcvd: None,
            report_sent: None,
            report_rcvd: None,
            tx_message: None,
            started_ms: 0,
            bye73_sent: false,
            no_reply_count: 0,
            cq_no_reply_count: 0,
            final_ack_sends: 0,
            dormant: None,
            answering_dormant_repeat: false,
        }
    }

    pub fn status(&self) -> QsoStatus {
        QsoStatus {
            active: self.active,
            stage: self.stage,
            target: self.target.clone(),
            tx_message: self.tx_message.clone(),
            report_sent: self.report_sent,
            report_rcvd: self.report_rcvd,
        }
    }

    pub fn tx_message(&self) -> Option<&str> {
        self.tx_message.as_deref()
    }

    /// The scheduler calls this after the current `tx_message` has actually been
    /// handed to the transmitter. It only matters for the courtesy "73": it marks
    /// the Bye73 as sent so the next `process_rx` may wrap the QSO up. For every
    /// other stage the sequencer advances on received replies, so this is a no-op.
    pub fn notify_transmitted(&mut self) {
        if self.stage == TxStage::Bye73 {
            self.bye73_sent = true;
        }
        // Count actual RR73/73 sends for max_73_sends -- see that field's
        // doc comment. Bye73 is already inherently bounded to one retransmit
        // (process_rx finishes the QSO the very next cycle once bye73_sent),
        // so only Rr73's repeat loop ever reaches the cap in practice, but
        // both are counted for parity with Android's own order-4/5 counter.
        if matches!(self.stage, TxStage::Rr73 | TxStage::Bye73) {
            self.final_ack_sends = self.final_ack_sends.saturating_add(1);
        }
    }

    /// Begin calling CQ.
    pub fn start_cq(&mut self) {
        self.reset_qso();
        self.dormant = None; // a deliberate new CQ abandons any repeat-answer window
        self.active = true;
        self.stage = TxStage::Cq;
        self.tx_message = Some(format!("CQ {} {}", self.my_call, self.my_grid));
    }

    /// Begin answering a specific decoded CQ (operator tapped a station).
    pub fn answer(&mut self, msg: &DecodedMessage) {
        self.reset_qso();
        self.dormant = None; // a deliberate new target abandons any repeat-answer window
        self.active = true;
        self.target = Some(msg.call_from.to_uppercase());
        self.grid_rcvd = if msg.grid.is_empty() { None } else { Some(msg.grid.clone()) };
        self.report_sent = Some(clamp_report(msg.snr));
        self.stage = TxStage::Grid;
        self.started_ms = util::now_unix_ms();
        self.tx_message = Some(format!(
            "{} {} {}",
            self.target.as_ref().unwrap(),
            self.my_call,
            self.my_grid
        ));
    }

    /// Operator manually picks which standard message to send next (the
    /// WSJT-X Tx1–Tx5 buttons). Requires an active target — it rebuilds
    /// `tx_message` for `stage` from the current QSO context, and the
    /// auto-sequencer then carries on from there on the next decoded reply.
    /// Selecting `Cq`/`Idle` falls through to start-CQ / stop so the same
    /// control can cover the whole strip.
    pub fn set_stage(&mut self, stage: TxStage) {
        match stage {
            TxStage::Cq => return self.start_cq(),
            TxStage::Idle => return self.stop(),
            _ => {}
        }
        let dx = match self.target.clone() {
            Some(t) => t,
            None => return, // no station selected yet — nothing to address
        };
        self.active = true;
        match stage {
            TxStage::Grid => {
                self.tx_message = Some(format!("{} {} {}", dx, self.my_call, self.my_grid));
                self.stage = TxStage::Grid;
            }
            TxStage::Report => self.send_report(&dx),
            TxStage::RReport => self.send_r_report(&dx),
            TxStage::Rr73 => self.send_rr73(&dx),
            TxStage::Bye73 => {
                self.tx_message = Some(format!("{} {} 73", dx, self.my_call));
                self.stage = TxStage::Bye73;
                self.bye73_sent = false;
            }
            TxStage::Cq | TxStage::Idle => unreachable!("handled above"),
        }
    }

    /// Send a free-text / arbitrary message once.
    pub fn set_free_text(&mut self, text: &str) {
        self.active = true;
        self.tx_message = Some(text.to_uppercase());
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.tx_message = None;
        self.stage = TxStage::Idle;
        // Clear the QSO context too, so a later manual set_stage() can't
        // re-activate and transmit to a stale target/report from the QSO we
        // just stopped (mirrors the iOS QsoEngine.stop()).
        self.reset_qso();
    }

    fn reset_qso(&mut self) {
        self.target = None;
        self.grid_rcvd = None;
        self.report_sent = None;
        self.report_rcvd = None;
        self.started_ms = util::now_unix_ms();
        self.bye73_sent = false;
        self.no_reply_count = 0;
        self.cq_no_reply_count = 0;
        self.final_ack_sends = 0;
    }

    /// Give up on the current target without logging a QSO -- the TX
    /// Watchdog's no-reply limit or the Max 73 Sends cap was hit. Mirrors
    /// `finish_qso`'s un-logged shape: returns to CQ when `auto_return_to_cq`,
    /// otherwise goes idle, same as Android's own "move on" behavior rather
    /// than hanging on a station that stopped answering.
    fn give_up_target(&mut self) {
        self.answering_dormant_repeat = false;
        self.reset_qso();
        if self.auto_return_to_cq {
            self.stage = TxStage::Cq;
            self.tx_message = Some(format!("CQ {} {}", self.my_call, self.my_grid));
        } else {
            self.active = false;
            self.stage = TxStage::Idle;
            self.tx_message = None;
        }
    }

    /// Process the decoded messages of a finished RX slot. Updates `tx_message`
    /// for the next TX slot and returns a completion outcome if a QSO finished.
    pub fn process_rx(&mut self, messages: &[DecodedMessage]) -> Option<QsoOutcome> {
        if !self.active || self.stage == TxStage::Idle {
            // Dormant repeat-answer window (see `dormant`'s doc comment):
            // TX is off after a clean completion, but a repeat from that
            // same station within the window still gets answered.
            if !self.active && self.dormant.is_some() {
                self.try_answer_dormant_repeat(messages);
            }
            return None;
        }

        // A courtesy "73" is queued (RReport -> RR73, or a manual Tx5). Only wrap
        // up (return to CQ / stop) once it has *actually* been transmitted — the
        // scheduler signals that via `notify_transmitted`. If it hasn't gone out
        // yet (missed TX window, rig busy, encode failure), leave the message
        // queued so the scheduler retransmits it next slot instead of clobbering
        // it. Either way we don't act on incoming traffic while a 73 is pending.
        if self.stage == TxStage::Bye73 {
            if self.bye73_sent {
                self.finish_qso();
            }
            return None;
        }

        // Find a message addressed to us, from our target if we have one.
        // crate::callsign::is_my_callsign (not a plain equality) so a
        // portable/compound call_to (e.g. "W1AW/P" when my_call is "W1AW")
        // is still recognized mid-QSO, matching Android's
        // GeneralVariables.checkIsMyCallsign.
        let mine = messages.iter().find(|m| {
            crate::callsign::is_my_callsign(&m.call_to, &self.my_call)
                && match &self.target {
                    Some(t) => m.call_from.eq_ignore_ascii_case(t),
                    None => true,
                }
        });

        let msg = match mine {
            Some(m) => m.clone(),
            None => {
                // No reply at all this slot -- same as a non-advancing
                // reply, see `advanced` below.
                self.on_no_progress();
                return None; // scheduler retransmits (or we just gave up above)
            }
        };

        if self.target.is_none() {
            self.target = Some(msg.call_from.to_uppercase());
            self.started_ms = util::now_unix_ms();
        }
        let dx = self.target.clone().unwrap();
        let content = classify(&msg);

        // Whether this cycle's content actually ADVANCES the stage, vs. a
        // stale/non-matching reply (the partner repeating what he already
        // sent because he isn't copying us -- Android's `isPartnerRepeat`).
        // Only a real advance resets the TX Watchdog's silence counter;
        // Android's own `noReplyCount` resets on *progress*, not on mere
        // presence of a message from the target -- a repeat counts as a
        // failed attempt, same as silence, for every stage that has a
        // give-up concept (everything except Cq, which has no specific
        // target yet to give up on).
        let advanced = matches!(
            (self.stage, &content),
            (TxStage::Cq, Content::Grid(_) | Content::Report(_))
                | (TxStage::Grid, Content::Report(_) | Content::RReport(_))
                | (TxStage::Report, Content::RReport(_) | Content::Rr73 | Content::Rrr | Content::Bye73)
                | (TxStage::RReport, Content::Rr73 | Content::Rrr | Content::Bye73)
                | (TxStage::Rr73, Content::Bye73 | Content::Rr73 | Content::Rrr)
        );
        if advanced {
            self.no_reply_count = 0;
            self.cq_no_reply_count = 0;
        } else {
            self.on_no_progress();
            return None;
        }

        match self.stage {
            TxStage::Cq => match content {
                Content::Grid(g) => {
                    self.grid_rcvd = Some(g);
                    self.report_sent = Some(clamp_report(msg.snr));
                    self.send_report(&dx);
                }
                Content::Report(n) => {
                    self.report_rcvd = Some(n);
                    self.report_sent = Some(clamp_report(msg.snr));
                    self.send_r_report(&dx);
                }
                _ => {}
            },
            TxStage::Grid => match content {
                Content::Report(n) => {
                    self.report_rcvd = Some(n);
                    // Our report reflects the DX signal from the decode we're
                    // answering (matches WSJT-X recomputing per decode).
                    self.report_sent = Some(clamp_report(msg.snr));
                    self.send_r_report(&dx);
                }
                Content::RReport(n) => {
                    self.report_rcvd = Some(n);
                    self.send_rr73(&dx);
                }
                _ => {}
            },
            TxStage::Report => match content {
                Content::RReport(n) => {
                    self.report_rcvd = Some(n);
                    self.send_rr73(&dx);
                }
                Content::Rr73 | Content::Rrr | Content::Bye73 => {
                    return self.complete(&dx);
                }
                _ => {}
            },
            TxStage::RReport => match content {
                Content::Rr73 | Content::Rrr | Content::Bye73 => {
                    // Acknowledge with 73, then log.
                    self.tx_message = Some(format!("{} {} 73", dx, self.my_call));
                    self.stage = TxStage::Bye73;
                    self.bye73_sent = false;
                    return self.complete(&dx);
                }
                _ => {}
            },
            TxStage::Rr73 => match content {
                Content::Bye73 | Content::Rr73 | Content::Rrr => {
                    return self.complete(&dx);
                }
                _ => {}
            },
            TxStage::Bye73 | TxStage::Idle => {}
        }
        None
    }

    /// Shared by total silence and a non-advancing reply (see `advanced`
    /// in `process_rx`): advances whichever give-up counter applies to
    /// the current stage. Checked every time progress didn't happen, not
    /// just on total silence -- Android's `hasReachedMax73Sends` runs
    /// every cycle for exactly this reason: a partner who keeps replying
    /// with something that never confirms (e.g. re-sending his own
    /// report) would otherwise never trip the cap, since presence of
    /// *any* message used to keep resetting the counter.
    fn on_no_progress(&mut self) {
        match self.stage {
            TxStage::Grid | TxStage::Report | TxStage::RReport => {
                self.no_reply_count = self.no_reply_count.saturating_add(1);
                if should_give_up_target(self.no_reply_limit, self.no_reply_count) {
                    self.give_up_target();
                }
            }
            TxStage::Rr73 => {
                let unlimited_failsafe =
                    self.max_73_sends == 0 && self.final_ack_sends >= RR73_GIVEUP_CYCLES_FAILSAFE;
                if has_reached_max_73_sends(self.max_73_sends, self.final_ack_sends) || unlimited_failsafe {
                    self.give_up_target();
                }
            }
            TxStage::Cq => {
                self.cq_no_reply_count = self.cq_no_reply_count.saturating_add(1);
                if should_give_up_cq(self.cq_no_reply_limit, self.cq_no_reply_count) {
                    // No target to fall back to -- this IS the CQ itself
                    // giving up, same as the operator pressing Stop.
                    self.stop();
                }
            }
            TxStage::Bye73 | TxStage::Idle => {}
        }
    }

    /// Android's `answerRepeatAfterLog`: TX is dormant after cleanly
    /// completing a QSO with `auto_return_to_cq` off, and the SAME
    /// station (same band, within `SAME_QSO_WINDOW_MS`) repeats their
    /// R-report or RR73 -- they missed our final RR73/73. Re-arm and
    /// answer the next stage as if the QSO were still open, reusing the
    /// ORIGINAL start time (`keepsQsoStart`, never a fresh one) and
    /// without logging a second `QsoRecord`. Capped at
    /// `MAX_REPEAT_ANSWERS`. Returns whether a repeat answer was armed.
    fn try_answer_dormant_repeat(&mut self, messages: &[DecodedMessage]) -> bool {
        let Some(dormant) = self.dormant.clone() else { return false };
        if dormant.repeat_answers >= MAX_REPEAT_ANSWERS {
            return false;
        }
        if util::now_unix_ms() - dormant.ended_ms >= SAME_QSO_WINDOW_MS {
            self.dormant = None;
            return false;
        }
        if self.band != dormant.band {
            return false;
        }

        let found = messages
            .iter()
            .find(|m| crate::callsign::is_my_callsign(&m.call_to, &self.my_call) && m.call_from.eq_ignore_ascii_case(&dormant.call));
        let Some(m) = found else { return false };
        let content = classify(m);
        if !matches!(content, Content::RReport(_) | Content::Rr73 | Content::Rrr | Content::Bye73) {
            return false;
        }

        self.target = Some(dormant.call.clone());
        self.started_ms = dormant.started_ms; // keepsQsoStart
        self.active = true;
        self.answering_dormant_repeat = true;
        self.no_reply_count = 0;
        if let Some(d) = self.dormant.as_mut() {
            d.repeat_answers += 1;
        }
        let dx = dormant.call.clone();
        match content {
            Content::RReport(n) => {
                self.report_rcvd = Some(n);
                self.send_rr73(&dx);
            }
            Content::Rr73 | Content::Rrr | Content::Bye73 => {
                self.tx_message = Some(format!("{} {} 73", dx, self.my_call));
                self.stage = TxStage::Bye73;
                self.bye73_sent = false;
                self.complete(&dx);
            }
            _ => unreachable!("guarded above"),
        }
        true
    }

    fn send_report(&mut self, dx: &str) {
        let r = self.report_sent.unwrap_or(0);
        self.tx_message = Some(format!("{} {} {}", dx, self.my_call, fmt_report(r)));
        self.stage = TxStage::Report;
    }

    fn send_r_report(&mut self, dx: &str) {
        let r = self.report_sent.unwrap_or(0);
        self.tx_message = Some(format!("{} {} R{}", dx, self.my_call, fmt_report(r)));
        self.stage = TxStage::RReport;
    }

    fn send_rr73(&mut self, dx: &str) {
        self.tx_message = Some(format!("{} {} RR73", dx, self.my_call));
        self.stage = TxStage::Rr73;
    }

    fn complete(&mut self, dx: &str) -> Option<QsoOutcome> {
        let record = QsoRecord {
            id: None,
            call: dx.to_string(),
            gridsquare: self.grid_rcvd.clone().unwrap_or_default(),
            mode: self.mode.clone(),
            rst_sent: self.report_sent.map(fmt_report).unwrap_or_default(),
            rst_rcvd: self.report_rcvd.map(fmt_report).unwrap_or_default(),
            qso_date: util::yyyymmdd_from_ms(self.started_ms),
            time_on: util::hhmmss_from_ms(self.started_ms),
            qso_date_off: util::utc_yyyymmdd(),
            time_off: util::utc_hhmmss(),
            band: self.band.clone(),
            freq: self.freq_mhz.clone(),
            station_callsign: self.my_call.clone(),
            my_gridsquare: self.my_grid.clone(),
            comment: String::new(),
            confirmed: false,
            ..Default::default()
        };

        // If a courtesy "73" is queued (the RReport arm set stage = Bye73), leave
        // tx_message/stage so the scheduler transmits it; the next slot's Bye73
        // handler in process_rx then wraps up. Otherwise finish now.
        if self.stage != TxStage::Bye73 {
            self.finish_qso();
        }
        // A dormant-repeat completion (see `try_answer_dormant_repeat`) is
        // the SAME QSO as the one already logged -- never return a second
        // loggable outcome for it, matching Android's "updateQSlRecordList
        // skips a just-logged station."
        if self.answering_dormant_repeat {
            None
        } else {
            Some(QsoOutcome::Completed(record))
        }
    }

    /// Return to calling CQ, or stop, per `auto_return_to_cq`.
    fn finish_qso(&mut self) {
        // Remember this station/band/start-time briefly (see `dormant`'s
        // doc comment) so a repeat within the window still gets answered,
        // without logging again -- only when auto_return_to_cq is off,
        // matching Android's own "TX switched itself off after a logged
        // QSO" precondition for this mechanism. Refreshes (not resets)
        // repeat_answers so a chain of repeats still respects the overall
        // MAX_REPEAT_ANSWERS cap across the whole window, not per-attempt.
        if !self.auto_return_to_cq {
            if let Some(target) = self.target.clone() {
                let repeat_answers = self.dormant.as_ref().map(|d| d.repeat_answers).unwrap_or(0);
                self.dormant = Some(DormantQso {
                    call: target,
                    band: self.band.clone(),
                    started_ms: self.started_ms,
                    ended_ms: util::now_unix_ms(),
                    repeat_answers,
                });
            }
        }
        self.answering_dormant_repeat = false;
        if self.auto_return_to_cq {
            self.reset_qso();
            self.stage = TxStage::Cq;
            self.tx_message = Some(format!("CQ {} {}", self.my_call, self.my_grid));
        } else {
            self.stop();
        }
    }
}

/// TX Watchdog ("Stop After" N unanswered attempts). Ported in spirit from
/// Android's `FT8TransmitSignal.shouldGiveUpTarget` -- desktop's stage model
/// differs enough (7 named `TxStage`s vs. Android's 6 numeric
/// `functionOrder`s) that this re-derives the same "silence since the
/// target last replied" signal natively via `no_reply_count` rather than
/// porting Android's order-number arithmetic line for line. `0` = unlimited
/// (Android's own `noReplyLimit=0` default) -- the operator's own Stop/
/// Hunt-rotate remains the ultimate backstop, matching Android's documented
/// reliance on its separate TX-supervision timeout for the unlimited case.
pub fn should_give_up_target(no_reply_limit: u32, no_reply_count: u32) -> bool {
    no_reply_limit > 0 && no_reply_count >= no_reply_limit
}

/// CQ Watchdog cap -- see `QsoEngine::cq_no_reply_limit`'s doc comment for
/// why this is a separate setting/counter from `should_give_up_target`'s.
/// `0` = unlimited, same convention, so an unattended/"robot" CQ operation
/// is unaffected unless the operator explicitly opts into a cap.
pub fn should_give_up_cq(cq_no_reply_limit: u32, cq_no_reply_count: u32) -> bool {
    cq_no_reply_limit > 0 && cq_no_reply_count >= cq_no_reply_limit
}

/// Max 73 Sends cap. Ported in spirit from Android's
/// `FT8TransmitSignal.hasReachedMax73Sends`. `0` = Auto/off (Android's own
/// default): no cap, just the `RR73_GIVEUP_CYCLES_FAILSAFE` safety net below.
pub fn has_reached_max_73_sends(max_73_sends: u32, final_ack_sends: u32) -> bool {
    max_73_sends > 0 && final_ack_sends >= max_73_sends
}

/// Hard failsafe for the `Rr73` retry loop when Max 73 Sends is left at
/// Auto (`0`): never hold the run frequency on a target that stopped
/// replying forever. Matches Android's own `RR73_GIVEUP_CYCLES` constant.
pub const RR73_GIVEUP_CYCLES_FAILSAFE: u32 = 3;

/// FT8 reports are clamped to roughly [-30, +30].
fn clamp_report(snr: i32) -> i32 {
    snr.clamp(-30, 30)
}

/// Format a report as a signed 2+ digit string: -9 -> "-09", 5 -> "+05".
fn fmt_report(n: i32) -> String {
    format!("{:+03}", n)
}

fn classify(msg: &DecodedMessage) -> Content {
    // Recognize the QSO sign-offs before treating the message as a grid report.
    // "RR73" is a valid 4-char Maidenhead-shaped token (R,R in A..R + two
    // digits), so the decoder copies it into `grid` (looks_like_grid("RR73") is
    // true). Checking grid first would classify a partner's standard RR73
    // sign-off as Content::Grid, stranding the auto-sequencer at the RReport
    // stage — it would retransmit our R-report forever and never complete/log
    // the QSO. Matching the sign-offs first fixes that without touching
    // looks_like_grid (the map/distance code relies on its shape test).
    let extra = msg.extra.trim();
    match extra {
        "RR73" => return Content::Rr73,
        "RRR" => return Content::Rrr,
        "73" => return Content::Bye73,
        _ => {}
    }
    if !msg.grid.is_empty() {
        return Content::Grid(msg.grid.clone());
    }
    if let Some(rest) = extra.strip_prefix('R') {
        if let Ok(n) = rest.parse::<i32>() {
            return Content::RReport(n);
        }
    }
    if let Ok(n) = extra.parse::<i32>() {
        return Content::Report(n);
    }
    Content::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(to: &str, from: &str, extra: &str, grid: &str, snr: i32) -> DecodedMessage {
        DecodedMessage {
            call_to: to.into(),
            call_from: from.into(),
            extra: extra.into(),
            grid: grid.into(),
            raw_text: format!("{to} {from} {extra}"),
            snr,
            freq_hz: 1500.0,
            time_sec: 0.5,
            score: 30,
            i3: 1,
            n3: 0,
            a91: [0; 12],
        }
    }

    #[test]
    fn answerer_full_sequence_logs_qso() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        // Operator answers DX K1ABC who CQ'd from FN42.
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ EN37"));
        assert_eq!(e.status().stage, TxStage::Grid);

        // DX reports our signal -12.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        assert!(out.is_none());
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ R-08"));
        assert_eq!(e.status().report_rcvd, Some(-12));

        // DX sends RR73 -> we log AND queue the courtesy 73 to transmit. The
        // decoder copies the grid-shaped "RR73" into the grid field, so pass it
        // here as it actually arrives live (grid == "RR73"), not grid == "".
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        match out {
            Some(QsoOutcome::Completed(rec)) => {
                assert_eq!(rec.call, "K1ABC");
                assert_eq!(rec.gridsquare, "FN42");
                assert_eq!(rec.rst_sent, "-08");
                assert_eq!(rec.rst_rcvd, "-12");
            }
            _ => panic!("expected completed QSO"),
        }
        // The final 73 is queued (not clobbered by the auto-return to CQ)...
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);
        // ...the scheduler actually transmits it (signalled here)...
        e.notify_transmitted();
        // ...and once its TX slot has passed, the next slot returns to CQ.
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn bye73_not_finished_until_actually_transmitted() {
        // Regression: process_rx must not wrap up on stage==Bye73 alone. If the
        // courtesy 73 couldn't be transmitted this slot (late TX window, rig busy,
        // encode failure) the QSO must stay armed and retransmit it, not clobber
        // it by returning to CQ.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ R-08"));

        // DX sends RR73 -> QSO logs and the courtesy 73 is queued. As live, the
        // grid-shaped "RR73" also lands in the decoder's grid field.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        assert!(matches!(out, Some(QsoOutcome::Completed(_))));
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);

        // The 73 did NOT go out this slot (no notify_transmitted). The next slot
        // must keep the 73 queued rather than returning to CQ, and must not log
        // the QSO a second time.
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);
        // Still pending after another silent slot.
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);

        // Once the scheduler reports the 73 actually transmitted, the next slot
        // wraps the QSO up and returns to CQ.
        e.notify_transmitted();
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn notify_transmitted_is_noop_off_bye73() {
        // notify_transmitted only latches the courtesy 73; it must not disturb an
        // in-progress sequence (e.g. after answering, stage == Grid).
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);
        e.notify_transmitted();
        // Still awaiting the DX report; nothing changed.
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ EN37"));
        assert_eq!(e.status().stage, TxStage::Grid);
        // The DX report still advances us normally.
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ R-08"));
        assert_eq!(e.status().stage, TxStage::RReport);
    }

    #[test]
    fn initiator_cq_sequence_logs_qso() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.start_cq();
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));

        // K1ABC answers our CQ with their grid.
        e.process_rx(&[msg("K0XYZ", "K1ABC", "FN42", "FN42", -10)]);
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ -10"));
        assert_eq!(e.status().stage, TxStage::Report);

        // K1ABC sends R-report.
        e.process_rx(&[msg("K0XYZ", "K1ABC", "R-15", "", -10)]);
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ RR73"));
        assert_eq!(e.status().report_rcvd, Some(-15));

        // K1ABC sends 73 -> complete, log, back to CQ.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "73", "", -10)]);
        match out {
            Some(QsoOutcome::Completed(rec)) => {
                assert_eq!(rec.call, "K1ABC");
                assert_eq!(rec.rst_sent, "-10");
                assert_eq!(rec.rst_rcvd, "-15");
            }
            _ => panic!("expected completed QSO"),
        }
        // auto-returned to CQ
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
    }

    #[test]
    fn stop_clears_target_so_set_stage_cannot_rekey_a_stopped_qso() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        // Answering a CQ arms a target + report context.
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().target.as_deref(), Some("K1ABC"));
        assert_eq!(e.status().report_sent, Some(-5));

        // Operator halts TX: the QSO context must be cleared, not just idled.
        e.stop();
        assert!(!e.status().active);
        assert_eq!(e.status().stage, TxStage::Idle);
        assert_eq!(e.status().target, None);
        assert_eq!(e.status().report_sent, None);
        assert_eq!(e.status().report_rcvd, None);

        // A later manual stage button (WSJT-X Tx1–Tx5) must NOT re-key the
        // just-stopped station — with no target, set_stage is a no-op.
        e.set_stage(TxStage::Report);
        assert!(!e.status().active);
        assert_eq!(e.tx_message(), None);
    }

    #[test]
    fn ignores_messages_for_others() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.start_cq();
        // traffic between two other stations
        assert!(e.process_rx(&[msg("W1AW", "K1ABC", "FN42", "FN42", -3)]).is_none());
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
    }

    #[test]
    fn rr73_signoff_is_not_classified_as_a_grid() {
        // "RR73" passes looks_like_grid, so the decoder copies it into the grid
        // field. classify must still treat it as the RR73 sign-off, not a grid
        // report — otherwise the auto-sequencer never completes the QSO.
        let signoff = msg("K0XYZ", "K1ABC", "RR73", "RR73", -8);
        assert!(matches!(classify(&signoff), Content::Rr73));
        // A genuine grid report is unaffected.
        let grid = msg("K0XYZ", "K1ABC", "FN42", "FN42", -8);
        assert!(matches!(classify(&grid), Content::Grid(g) if g == "FN42"));
    }

    #[test]
    fn rr73_with_decoder_grid_still_completes_qso() {
        // Regression for the sign-off-vs-grid classification bug. Live, the DX's
        // RR73 arrives with grid == "RR73" (looks_like_grid is true). Before the
        // fix classify returned Content::Grid, the RReport arm hit `_ => {}`, and
        // the QSO hung retransmitting our R-report forever instead of logging.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ R-08"));
        assert_eq!(e.status().stage, TxStage::RReport);

        // DX signs off with RR73 exactly as the decoder emits it live.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        match out {
            Some(QsoOutcome::Completed(rec)) => {
                assert_eq!(rec.call, "K1ABC");
                assert_eq!(rec.gridsquare, "FN42");
                assert_eq!(rec.rst_sent, "-08");
                assert_eq!(rec.rst_rcvd, "-12");
            }
            _ => panic!("RR73 sign-off must complete the QSO"),
        }
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);
    }

    // --- TX Watchdog (no_reply_limit) / Max 73 Sends (max_73_sends) ---

    #[test]
    fn should_give_up_target_pure_cases() {
        assert!(!should_give_up_target(0, 1000)); // unlimited never gives up here
        assert!(!should_give_up_target(5, 4));
        assert!(should_give_up_target(5, 5));
        assert!(should_give_up_target(5, 6));
    }

    #[test]
    fn has_reached_max_73_sends_pure_cases() {
        assert!(!has_reached_max_73_sends(0, 1000)); // Auto/off never caps here
        assert!(!has_reached_max_73_sends(3, 2));
        assert!(has_reached_max_73_sends(3, 3));
    }

    // --- CQ Watchdog (cq_no_reply_limit) ---

    #[test]
    fn should_give_up_cq_pure_cases() {
        assert!(!should_give_up_cq(0, 1000)); // unlimited -- robot/unattended CQ never gives up
        assert!(!should_give_up_cq(5, 4));
        assert!(should_give_up_cq(5, 5));
        assert!(should_give_up_cq(5, 6));
    }

    #[test]
    fn cq_watchdog_stops_calling_cq_after_n_silent_cycles() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.cq_no_reply_limit = 2;
        e.start_cq();
        assert!(e.status().active);
        assert_eq!(e.status().stage, TxStage::Cq);

        // First silent cycle -- not given up yet, still calling.
        assert!(e.process_rx(&[]).is_none());
        assert!(e.status().active);
        assert_eq!(e.status().stage, TxStage::Cq);

        // Second silent cycle hits the limit -- stops entirely (no target to
        // fall back to, unlike the mid-QSO TX Watchdog's "return to CQ").
        assert!(e.process_rx(&[]).is_none());
        assert!(!e.status().active);
        assert_eq!(e.status().stage, TxStage::Idle);
        assert_eq!(e.tx_message(), None);
    }

    #[test]
    fn cq_watchdog_disabled_calls_cq_forever() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.cq_no_reply_limit = 0; // the "robot operation" default
        e.start_cq();
        for _ in 0..200 {
            assert!(e.process_rx(&[]).is_none());
        }
        assert!(e.status().active, "unlimited CQ Watchdog must never give up");
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn a_real_reply_to_cq_resets_the_cq_watchdog_and_does_not_count_against_it() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.cq_no_reply_limit = 2;
        e.start_cq();
        assert!(e.process_rx(&[]).is_none()); // 1 silent cycle, one short of the cap

        // A real answer to our CQ (a grid report) advances us into a QSO
        // with that target, replying with our own signal report right
        // away -- must reset the CQ-specific counter, not merely leave Cq
        // stage.
        e.process_rx(&[msg("K0XYZ", "K1ABC", "FN42", "FN42", -5)]);
        assert_eq!(e.status().stage, TxStage::Report);
        assert!(e.status().active);
    }

    #[test]
    fn cq_watchdog_is_independent_of_the_mid_qso_tx_watchdog() {
        // A tight mid-QSO retry tolerance must not shorten how long a bare,
        // unanswered CQ is allowed to run -- these are deliberately separate
        // counters/limits (see cq_no_reply_limit's doc comment).
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 1; // give up on a silent target almost immediately
        e.cq_no_reply_limit = 0; // but let a bare CQ run unlimited
        e.start_cq();
        for _ in 0..10 {
            assert!(e.process_rx(&[]).is_none());
        }
        assert!(e.status().active, "no_reply_limit must not leak into the Cq stage");
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn tx_watchdog_gives_up_a_silent_target_and_returns_to_cq() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 2;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);

        // Two silent slots -- not given up yet.
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.status().stage, TxStage::Grid);
        assert!(e.process_rx(&[]).is_none());
        // The 2nd silent slot hits the limit and gives up -> back to CQ.
        assert_eq!(e.status().stage, TxStage::Cq);
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
    }

    #[test]
    fn tx_watchdog_disabled_never_gives_up() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 0;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        for _ in 0..50 {
            e.process_rx(&[]);
        }
        assert_eq!(e.status().stage, TxStage::Grid);
    }

    #[test]
    fn tx_watchdog_disabled_without_auto_return_to_cq_goes_idle() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.auto_return_to_cq = false;
        e.no_reply_limit = 1;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[]);
        assert_eq!(e.status().stage, TxStage::Idle);
        assert!(!e.active);
        assert_eq!(e.tx_message(), None);
    }

    #[test]
    fn a_reply_resets_the_no_reply_counter() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 2;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[]); // 1 silent slot
        // A real reply arrives before the limit -- must reset the counter,
        // not carry it over into the next stage.
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        assert_eq!(e.status().stage, TxStage::RReport);
        e.process_rx(&[]);
        assert_eq!(e.status().stage, TxStage::RReport); // 1 silent slot, not given up
        e.process_rx(&[]);
        // 2nd silent slot in THIS stage hits the limit (counter was reset by the reply).
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn max_73_sends_caps_rr73_retransmission_and_returns_to_cq() {
        // Drive a Grid -> RReport(n) -> send_rr73 -> Rr73 loop (RReport's
        // own RR73-reply path goes straight to Bye73 instead; Rr73 is the
        // stage that actually retransmits RR73 while the target is silent).
        let mut e2 = QsoEngine::new("K0XYZ", "EN37");
        e2.max_73_sends = 2;
        e2.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e2.process_rx(&[msg("K0XYZ", "K1ABC", "R-12", "", -8)]); // RReport -> send_rr73 -> Rr73
        assert_eq!(e2.status().stage, TxStage::Rr73);
        e2.notify_transmitted(); // 1st RR73 send
        e2.process_rx(&[]); // silent, not capped yet (1 < 2)
        assert_eq!(e2.status().stage, TxStage::Rr73);
        e2.notify_transmitted(); // 2nd RR73 send
        e2.process_rx(&[]); // silent, now capped (2 >= 2) -> give up
        assert_eq!(e2.status().stage, TxStage::Cq);
    }

    #[test]
    fn rr73_unlimited_failsafe_still_gives_up_eventually() {
        // max_73_sends left at Auto (0): the RR73_GIVEUP_CYCLES_FAILSAFE
        // safety net still ends the loop instead of holding the frequency
        // forever on a target that stopped replying.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "R-12", "", -8)]); // -> Rr73
        assert_eq!(e.status().stage, TxStage::Rr73);
        for _ in 0..RR73_GIVEUP_CYCLES_FAILSAFE {
            e.notify_transmitted();
            e.process_rx(&[]);
        }
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn max_73_sends_caps_even_when_the_partner_keeps_replying_without_confirming() {
        // Real bug fix: the cap used to be checked only on total silence.
        // A partner who keeps replying every cycle with something that
        // never confirms (e.g. re-sending his own report) must still
        // trip the cap, not hold the frequency forever.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.max_73_sends = 2;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "R-12", "", -8)]); // -> Rr73
        assert_eq!(e.status().stage, TxStage::Rr73);
        e.notify_transmitted(); // 1st RR73 send
        e.process_rx(&[msg("K0XYZ", "K1ABC", "R-12", "", -8)]); // still not confirming
        assert_eq!(e.status().stage, TxStage::Rr73); // not capped yet (1 < 2)
        e.notify_transmitted(); // 2nd RR73 send
        e.process_rx(&[msg("K0XYZ", "K1ABC", "R-12", "", -8)]); // still not confirming
        assert_eq!(e.status().stage, TxStage::Cq); // now capped -> give up
    }

    #[test]
    fn a_partner_repeat_counts_toward_the_watchdog_same_as_silence() {
        // Real bug fix: previously ANY message from the target reset the
        // no-reply counter, even a stale repeat that doesn't advance the
        // stage. A partner re-sending the same thing he already sent
        // (not copying us) must count as a failed attempt, same as
        // silence would.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 2;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);
        // He keeps re-sending his grid (what he already sent before we
        // even answered) instead of a report -- never advances Grid.
        e.process_rx(&[msg("K0XYZ", "K1ABC", "", "FN42", -8)]);
        assert_eq!(e.status().stage, TxStage::Grid); // 1 non-advancing reply
        e.process_rx(&[msg("K0XYZ", "K1ABC", "", "FN42", -8)]);
        // 2nd non-advancing reply hits the limit -> give up, same as silence would.
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn dormant_repeat_answers_without_logging_a_duplicate_qso() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.auto_return_to_cq = false; // Android's precondition: TX off after a logged QSO
        e.band = "20m".to_string();
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]); // -> RReport, R-08 sent
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]); // logs, queues 73
        assert!(matches!(out, Some(QsoOutcome::Completed(_))));
        e.notify_transmitted();
        assert!(e.process_rx(&[]).is_none()); // 73 sent -> TX stops (auto_return_to_cq=false)
        assert!(!e.status().active);
        assert_eq!(e.status().stage, TxStage::Idle);

        // K1ABC missed our 73 and repeats RR73 within the window.
        let out2 = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        assert!(out2.is_none(), "a repeat must never produce a second loggable outcome");
        assert!(e.status().active);
        assert_eq!(e.status().target.as_deref(), Some("K1ABC"));
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);

        // Transmitting the repeat 73 completes (still not logged) and
        // returns to the dormant/idle state.
        e.notify_transmitted();
        assert!(e.process_rx(&[]).is_none());
        assert!(!e.status().active);
    }

    #[test]
    fn dormant_repeat_is_capped_at_max_repeat_answers() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.auto_return_to_cq = false;
        e.band = "20m".to_string();
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        e.notify_transmitted();
        e.process_rx(&[]); // stopped, dormant set, repeat_answers=0

        for _ in 0..MAX_REPEAT_ANSWERS {
            assert!(e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]).is_none());
            assert!(e.status().active, "repeat answer must be armed under the cap");
            e.notify_transmitted();
            e.process_rx(&[]); // completes again, stops again
            assert!(!e.status().active);
        }

        // Cap reached -- the next repeat must NOT be re-armed.
        assert!(e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]).is_none());
        assert!(!e.status().active, "must stay idle once the repeat-answer cap is reached");
    }

    #[test]
    fn a_watchdog_give_up_never_creates_a_repeat_answer_window() {
        // Android's cappedCallsign intent ("the operator's limit wins"),
        // achieved structurally here: give_up_target never sets `dormant`,
        // so a station abandoned via the TX Watchdog is never eligible
        // for a repeat answer.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.auto_return_to_cq = false;
        e.no_reply_limit = 1;
        e.band = "20m".to_string();
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[]); // silent -> gives up (no_reply_limit=1), goes idle
        assert!(!e.status().active);

        // K1ABC shows up later with an RR73-shaped message -- must NOT be
        // treated as a repeat-answer window, since we gave up, never completed.
        assert!(e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]).is_none());
        assert!(!e.status().active, "a watchdog give-up must never be repeat-answered");
    }

    #[test]
    fn starting_a_new_cq_abandons_a_pending_repeat_answer_window() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.auto_return_to_cq = false;
        e.band = "20m".to_string();
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        e.notify_transmitted();
        e.process_rx(&[]); // stopped, dormant set for K1ABC

        e.start_cq(); // operator deliberately starts a fresh CQ -- clears dormant
        e.stop(); // and immediately stops again, back to idle with no live QSO

        // K1ABC's repeat must NOT be answered now -- the window from the
        // original QSO was abandoned the moment a new CQ was started.
        assert!(e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]).is_none());
        assert!(!e.status().active);
    }
}
