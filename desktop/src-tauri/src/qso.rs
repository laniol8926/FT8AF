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
    /// Other stations currently calling us while we're busy with `target` --
    /// see `QueuedCaller`'s own doc comment.
    pub caller_queue: Vec<QueuedCaller>,
}

/// A station other than our current target that called us (a directed
/// message naming our callsign) while a QSO was already in progress --
/// ported from Android's `QueuedCaller`/`FT8TransmitSignal.callerQueue`.
/// Unlike [`DormantQso`] (which only ever tracks OUR just-finished partner
/// for a repeat-73 courtesy answer), this tracks genuinely different
/// stations so the operator can jump to one without losing the locked
/// active-QSO panel view or interrupting the TX already in flight to the
/// current target. See `QsoEngine::offer_callers`'s doc comment for
/// exactly when an entry is added/refreshed/pruned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedCaller {
    pub callsign: String,
    pub grid: String,
    pub snr: i32,
    pub freq_hz: f32,
    pub extra: String,
    pub queued_at_ms: i64,
    pub last_heard_ms: i64,
}

/// What the engine wants the scheduler to do after processing a slot.
#[derive(Debug, Clone)]
pub enum QsoOutcome {
    /// A QSO finished and should be logged.
    Completed(QsoRecord),
    /// The TX Watchdog ("Stop After" N no-reply) or Max 73 Sends cap forced
    /// an end to the current target -- no record to log, but the engine
    /// still needs to know so it can revert any excursion-scoped state (see
    /// `Engine::preferred_tx_parity_restore`'s doc comment). Never produced
    /// by the bare-CQ give-up (`TxStage::Cq`'s branch of `on_no_progress`)
    /// -- that one has no target/answer excursion to unwind.
    GaveUpTarget,
}

#[derive(Debug, Clone, PartialEq)]
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
    /// noReplyLimit`/`shouldGiveUpTarget`, but deliberately NOT matching
    /// Android's "0 = unlimited" semantics -- see
    /// `effective_no_reply_limit`'s doc comment for why. Real callers
    /// floor this via that function before comparing (see
    /// `should_give_up_target`'s own doc comment for why it stays pure/
    /// unfloored itself).
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
    /// unrelated intents. `0` floors to `DEFAULT_CQ_WATCHDOG_LIMIT` (see
    /// `should_give_up_cq`'s doc comment) -- NOT unlimited, by deliberate
    /// policy: "we do not want this app to become a BOT... there must be a
    /// control operator present." A long stretch calling CQ is normal; an
    /// unattended/"robot" CQ running indefinitely is exactly what this
    /// floor exists to rule out.
    pub cq_no_reply_limit: u32,
    /// Settings -> Transmission's "CQ Answer Attempts" -- how many times to
    /// retry answering a CQ (`TxStage::Grid` retries specifically, i.e. he
    /// has never yet replied even once) before giving up, separate from
    /// `no_reply_limit`'s broader "retransmit mid-QSO" count. Real UX
    /// requirement, stated live: "while I'm in a QSO I'd want the TX
    /// Watchdog setting to control how many times I will retransmit a
    /// message. But if I'm replying to a station calling CQ how many times
    /// will I try to answer his CQ before giving up? No more than the TX
    /// Watchdog setting... want to avoid [a case like] the TX Watchdog
    /// being set to unlimited so I keep calling him forever." `0` floors
    /// to `DEFAULT_CQ_ANSWER_LIMIT` -- the "Same as TX Watchdog"
    /// deferred-to-`no_reply_limit` behavior this used to have was removed
    /// after it kept giving up after a single real attempt on a live
    /// Windows run (see that constant's doc comment). Never allowed to
    /// exceed `no_reply_limit` when finite (a CQ-answer cap higher than
    /// the general mid-QSO cap wouldn't make sense) -- see
    /// `effective_cq_answer_limit`.
    pub cq_answer_limit: u32,

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
    /// True once a `QsoOutcome::Completed` has been returned for the current
    /// target, by *any* path -- guards against a duplicate `QsoRecord` for
    /// the same contact. Two real bugs rolled into this one flag:
    ///
    /// 1. Found live: "I sent my RR73 but never received his 73. I should
    ///    have logged the QSO but did not." Standard FT8 practice (and
    ///    Android's own `updateQSlRecordList` order-4 "RR73 sent" save)
    ///    treats sending RR73 as sufficient to log the contact -- a reply
    ///    "73" is pure courtesy, never required. `complete()` used to only
    ///    ever fire on *receiving* a confirmation -- a partner who simply
    ///    never sends one (common) meant the QSO stayed unlogged forever
    ///    once Max 73 Sends ran out and `give_up_target` discarded it.
    /// 2. Reported live, already fixed once on the Android side: logging
    ///    used to fire the instant the *other* party's confirmation was
    ///    *received* -- i.e. before our own closing message (RR73 or the
    ///    courtesy 73) had actually been transmitted. If that transmission
    ///    then failed to go out (missed TX window, rig busy, encode
    ///    failure), the QSO was already logged even though the message
    ///    that's supposed to close it out was never sent.
    ///
    /// Both are fixed the same way: logging only ever happens from
    /// `notify_transmitted` (confirmed *sent*, not just queued), for
    /// *either* closing message -- RR73 (the caller's side) or the courtesy
    /// 73 (the answerer's side). `complete()` is now reached only when a
    /// LATER confirming reply arrives after logging already happened via
    /// `notify_transmitted` -- it just finishes the run without building a
    /// second record.
    already_logged: bool,
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
    /// See `QueuedCaller`'s doc comment. Deliberately NOT cleared by
    /// `reset_qso`/`answer` (switching to a different, possibly
    /// queue-sourced target must leave any OTHER queued callers intact for
    /// a later tap) -- only `stop()` and `start_cq()` clear it, matching
    /// Android's own clear call sites (TX deactivated, operator pressed CQ).
    caller_queue: Vec<QueuedCaller>,
}

/// Matches Android's `FT8TransmitSignal.MAX_QUEUE_SIZE`.
const MAX_CALLER_QUEUE: usize = 10;

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
            cq_answer_limit: 0,
            stage: TxStage::Idle,
            target: None,
            grid_rcvd: None,
            report_sent: None,
            report_rcvd: None,
            tx_message: None,
            started_ms: 0,
            bye73_sent: false,
            already_logged: false,
            no_reply_count: 0,
            cq_no_reply_count: 0,
            final_ack_sends: 0,
            dormant: None,
            answering_dormant_repeat: false,
            caller_queue: Vec::new(),
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
            caller_queue: self.caller_queue.clone(),
        }
    }

    pub fn tx_message(&self) -> Option<&str> {
        self.tx_message.as_deref()
    }

    /// The scheduler calls this after the current `tx_message` has actually been
    /// *handed to the transmitter* (keyed, not yet finished playing). For
    /// Bye73 it marks the courtesy "73" as sent. Every other stage advances
    /// on received replies, so this is a no-op there.
    ///
    /// Returns a loggable outcome the *first* time either closing message --
    /// RR73 (the caller's side) or the courtesy 73 (the answerer's side) --
    /// is confirmed sent for the current target. Real bug, ported from a fix
    /// already made on the Android side: logging used to fire the instant
    /// the *other* party's confirming message was *received* (`complete`,
    /// called right as the stage advances to Bye73/Rr73) -- before our own
    /// closing message had actually gone out. If that transmission then
    /// failed to send (missed TX window, rig busy, encode failure), the QSO
    /// was already logged even though the RR73/73 that's supposed to close
    /// it out was never actually sent. Logging now waits for this method --
    /// confirmed *sent*, not just queued -- exactly like Android's fix.
    /// See `already_logged`'s own doc comment for how a later, separate
    /// confirming reply is kept from logging a duplicate.
    pub fn notify_transmitted(&mut self) -> Option<QsoOutcome> {
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
        if matches!(self.stage, TxStage::Rr73 | TxStage::Bye73) {
            return self.log_if_not_yet();
        }
        None
    }

    /// Shared by `notify_transmitted`'s two closing-message branches: build
    /// and return a loggable outcome for the current target, exactly once.
    fn log_if_not_yet(&mut self) -> Option<QsoOutcome> {
        if self.already_logged || self.answering_dormant_repeat {
            return None;
        }
        let dx = self.target.clone().unwrap_or_default();
        let record = self.build_qso_record(&dx);
        self.already_logged = true;
        Some(QsoOutcome::Completed(record))
    }

    /// The scheduler calls this once the courtesy 73's transmission has
    /// actually *finished* keying (PTT dropped) -- `engine.rs`'s
    /// `finalize_tx`, the one place every TX mode (audio and CAT-tone)
    /// already detects "the waveform/symbols are done." Wraps the QSO up
    /// right then, instead of waiting for the next decoded slot's
    /// `process_rx` call to notice `bye73_sent` (up to ~30s later -- a real
    /// UX bug, found live: the operator missed 2-3 decode cycles after
    /// every single QSO while the docked panel stayed up with nothing left
    /// to do). A no-op unless we're genuinely still at Bye73 with the
    /// message already confirmed sent, so calling this after every TX
    /// completion (including ones that have nothing to do with a QSO, e.g.
    /// a bare CQ or Tune) is harmless. `process_rx`'s own Bye73 check is
    /// left in place as a fallback -- stage will simply no longer be Bye73
    /// by the time it runs, so it's a no-op there once this has already
    /// fired.
    pub fn finish_if_bye73_sent(&mut self) {
        if self.stage == TxStage::Bye73 && self.bye73_sent {
            self.finish_qso();
        }
    }

    /// Begin calling CQ.
    pub fn start_cq(&mut self) {
        self.reset_qso();
        self.dormant = None; // a deliberate new CQ abandons any repeat-answer window
        self.clear_caller_queue();
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

    /// Answer a station that called US (a directed message naming our own
    /// callsign -- a caller-queue chip, or any future "he's calling me,
    /// not the other way around" entry point), as opposed to [`answer`]
    /// (operator breaking into someone ELSE's CQ bulletin, where we have
    /// nothing from them yet and must send our own grid first). Real bug,
    /// found live: tapping a queued caller who had already sent us a
    /// grid-shaped message was sending "HISCALL MYCALL MYGRID" (`answer`'s
    /// always-Grid-stage reply) instead of "HISCALL MYCALL <report>" --
    /// wrong, since his message already told us what we need to send him
    /// a signal report immediately, same as `process_rx`'s own
    /// `TxStage::Cq` branch does the instant a bare CQ picks up its first
    /// responder (`self.target.is_none()` there). This reuses that exact
    /// same content-based jump (duplicated, not shared via a refactor of
    /// `process_rx`'s central dispatch, to keep this a narrow, low-risk
    /// fix) so a caller-queue tap resumes the conversation exactly where
    /// it actually is, not where a brand-new exchange would start.
    pub fn answer_caller(&mut self, msg: &DecodedMessage) {
        self.reset_qso();
        self.dormant = None; // a deliberate new target abandons any repeat-answer window
        self.active = true;
        self.target = Some(msg.call_from.to_uppercase());
        self.started_ms = util::now_unix_ms();
        let dx = self.target.clone().unwrap();
        match classify(msg) {
            Content::Grid(g) => {
                self.grid_rcvd = Some(g);
                self.report_sent = Some(clamp_report(msg.snr));
                self.send_report(&dx);
            }
            Content::Report(n) => {
                self.grid_rcvd = if msg.grid.is_empty() { None } else { Some(msg.grid.clone()) };
                self.report_rcvd = Some(n);
                self.report_sent = Some(clamp_report(msg.snr));
                self.send_r_report(&dx);
            }
            // Anything else (RReport/Rr73/Rrr/Bye73/Other -- an unusual
            // shape for a fresh caller-queue entry, see offer_callers'
            // own doc comment on what qualifies) -- fall back to the same
            // safe baseline `answer()` uses: send our own grid first
            // rather than guess a protocol jump this content doesn't
            // actually support.
            _ => {
                self.grid_rcvd = if msg.grid.is_empty() { None } else { Some(msg.grid.clone()) };
                self.report_sent = Some(clamp_report(msg.snr));
                self.stage = TxStage::Grid;
                self.tx_message = Some(format!("{dx} {} {}", self.my_call, self.my_grid));
            }
        }
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
        self.clear_caller_queue();
    }

    fn reset_qso(&mut self) {
        self.target = None;
        self.grid_rcvd = None;
        self.report_sent = None;
        self.report_rcvd = None;
        self.started_ms = util::now_unix_ms();
        self.bye73_sent = false;
        self.already_logged = false;
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

        // A courtesy "73" is queued (RReport -> RR73, or a manual Tx5). Normally
        // already wrapped up by now -- `finish_if_bye73_sent` fires the instant
        // the 73's transmission actually finishes (engine.rs's `finalize_tx`),
        // not here. This is the fallback for the one case that can't: if it
        // hasn't gone out yet (missed TX window, rig busy, encode failure),
        // leave the message queued so the scheduler retransmits it next slot
        // instead of clobbering it. Either way we don't act on incoming traffic
        // while a 73 is pending.
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
                // He's actively exchanging with someone else -- a real
                // signal he's moved on, not just silence. Give up right
                // away rather than waiting out the full TX Watchdog cycle
                // count -- see `target_working_someone_else`'s own doc
                // comment.
                if let Some(target) = self.target.clone() {
                    if let Some(m) = messages.iter().find(|m| {
                        m.call_from.eq_ignore_ascii_case(&target)
                            && !crate::callsign::is_cq_callsign(&m.call_to)
                            && !crate::callsign::is_my_callsign(&m.call_to, &self.my_call)
                    }) {
                        // TEMPORARY diagnostic (see log_watchdog_diag's own
                        // doc comment) -- a give-up with no preceding
                        // "Grid miss" log line can only mean it went
                        // through THIS immediate path, not the counted
                        // one. Logs the exact decode that triggered it so
                        // the next occurrence shows whether it's a real
                        // exchange with someone else or a garbled/
                        // misheard decode producing a false positive
                        // (this function's own doc comment already flags
                        // that risk).
                        log_watchdog_diag(&format!(
                            "{} target_working_someone_else target={} raw=\"{}\" call_from={} call_to={} snr={} score={}",
                            util::now_unix_ms(), target, m.raw_text, m.call_from, m.call_to, m.snr, m.score
                        ));
                        self.give_up_target();
                        return Some(QsoOutcome::GaveUpTarget);
                    }
                }
                // No reply at all this slot -- same as a non-advancing
                // reply, see `advanced` below.
                return if self.on_no_progress() {
                    Some(QsoOutcome::GaveUpTarget)
                } else {
                    None // scheduler retransmits
                };
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
        // sent because he isn't copying us).
        let advanced = matches!(
            (self.stage, &content),
            (TxStage::Cq, Content::Grid(_) | Content::Report(_))
                | (TxStage::Grid, Content::Report(_) | Content::RReport(_))
                | (TxStage::Report, Content::RReport(_) | Content::Rr73 | Content::Rrr | Content::Bye73)
                | (TxStage::RReport, Content::Rr73 | Content::Rrr | Content::Bye73)
                | (TxStage::Rr73, Content::Bye73 | Content::Rr73 | Content::Rrr)
        );
        // Reset on ANY genuine message from him -- he's clearly still
        // there, even when this cycle's content doesn't advance the stage
        // (e.g. a repeat, because he didn't copy our last reply). Real UX
        // requirement, stated live: "the [TX Watchdog] counter is
        // incremented each time i send a message but that is assuming no
        // response from the other station. If there happens to be a
        // response to a subsequent transmission the TX Watchdog counter
        // should be reinitialized... If he responds to me before the TX
        // Watchdog setting is reached I should get another 1 thru TX
        // Watchdog setting opportunity to reply to him" -- a live partner
        // repeating himself is a fundamentally different, much less
        // worrying situation than total silence (the only thing that
        // should actually count against either watchdog -- see the
        // `mine == None` branch above, which still increments). Previously
        // ported from Android's own `isPartnerRepeat`, which counts a
        // repeat as a failed attempt same as silence -- reverted here per
        // this live requirement; desktop's watchdogs now measure "has he
        // gone silent," not "has the exchange stalled."
        //
        // Rr73 is the one deliberate exception: its own cap (max_73_sends/
        // final_ack_sends, checked inside on_no_progress below) bounds how
        // many times WE resend our own closing message, regardless of
        // whether he's still there -- a different kind of safety net than
        // "has he gone silent," so it must still see every non-advancing
        // cycle, not just true silence.
        self.no_reply_count = 0;
        self.cq_no_reply_count = 0;
        if !advanced {
            if self.stage == TxStage::Rr73 {
                return if self.on_no_progress() {
                    Some(QsoOutcome::GaveUpTarget)
                } else {
                    None
                };
            }
            // Non-advancing but genuine reply (a repeat) -- re-send our
            // current message next slot, same as silence would, just
            // without counting against either watchdog.
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
                    // He jumped straight to confirming (skipped RReport, e.g.
                    // a decode glitch) -- still owe him the same courtesy 73
                    // RReport's own arm below sends; queue it the same way.
                    // Real bug, ported from a fix already made on the
                    // Android side (see notify_transmitted's own doc
                    // comment): don't log here, before that 73 has actually
                    // gone out -- only once notify_transmitted confirms it
                    // was really sent.
                    self.tx_message = Some(format!("{} {} 73", dx, self.my_call));
                    self.stage = TxStage::Bye73;
                    self.bye73_sent = false;
                }
                _ => {}
            },
            TxStage::RReport => match content {
                Content::Rr73 | Content::Rrr | Content::Bye73 => {
                    // Acknowledge with 73 -- notify_transmitted logs once
                    // it's actually sent, not before (see its own doc
                    // comment for the Android-side bug this ports a fix for).
                    self.tx_message = Some(format!("{} {} 73", dx, self.my_call));
                    self.stage = TxStage::Bye73;
                    self.bye73_sent = false;
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

    /// Scan a decoded slot for stations OTHER than our current target
    /// calling us, and queue/refresh/prune them -- ported from Android's
    /// `FT8TransmitSignal.refreshCallerQueue`. Deliberately a SEPARATE pass
    /// over `messages` from `process_rx`, called unconditionally every
    /// cycle regardless of stage: `process_rx` itself early-returns at
    /// `TxStage::Bye73` (see its own comment) before ever looking at
    /// `messages` for a third party, which is exactly why a station calling
    /// during our courtesy-73 retries was invisible before this existed
    /// (the live bug this ports Android's fix for).
    ///
    /// `cycle_ms` is the current mode's slot length (15 s FT8 / 7.5 s FT4,
    /// `Mode::cycle_ms()` in `engine.rs`) -- passed in rather than stored,
    /// so this pure state machine doesn't need to depend on `crate::mode`
    /// for what both other give-up timers already treat as an engine-side
    /// concern. Pruning uses "one full cycle without being heard again"
    /// (`2 * cycle_ms`, boundary inclusive), matching Android's
    /// `CallerQueueOrdering.hasGivenUp`.
    pub fn offer_callers(&mut self, messages: &[DecodedMessage], now_ms: i64, cycle_ms: i64) {
        if !self.active || self.target.is_none() {
            return;
        }
        let target = self.target.clone();
        for m in messages {
            if !crate::callsign::is_my_callsign(&m.call_to, &self.my_call) {
                continue;
            }
            if target.as_deref().is_some_and(|t| m.call_from.eq_ignore_ascii_case(t)) {
                continue; // our own QSO partner, not a third party
            }
            if classify(m) == Content::Bye73 {
                continue; // a bare courtesy "73" carries nothing worth queuing
            }
            let call = m.call_from.to_uppercase();
            if let Some(existing) = self.caller_queue.iter_mut().find(|q| q.callsign == call) {
                // Mirrors QueuedCaller.refresh: take the new SNR, never move
                // last_heard_ms backward (an evidence/deep pass can replay
                // an older slot's decode after a newer one already counted).
                existing.snr = m.snr;
                existing.grid = m.grid.clone();
                existing.extra = m.extra.clone();
                existing.freq_hz = m.freq_hz;
                existing.last_heard_ms = existing.last_heard_ms.max(now_ms);
            } else if self.caller_queue.len() < MAX_CALLER_QUEUE {
                self.caller_queue.push(QueuedCaller {
                    callsign: call,
                    grid: m.grid.clone(),
                    snr: m.snr,
                    freq_hz: m.freq_hz,
                    extra: m.extra.clone(),
                    queued_at_ms: now_ms,
                    last_heard_ms: now_ms,
                });
            }
        }
        self.prune_given_up_callers(now_ms, cycle_ms);
    }

    fn prune_given_up_callers(&mut self, now_ms: i64, cycle_ms: i64) {
        let max_idle_ms = 2 * cycle_ms;
        self.caller_queue.retain(|q| now_ms - q.last_heard_ms <= max_idle_ms);
    }

    /// Remove and return a specific queued caller regardless of staleness --
    /// the operator's own explicit tap, which should always be honored
    /// (matches Android's `dequeueSpecificCaller` deliberately skipping the
    /// give-up prune).
    pub fn dequeue_specific(&mut self, callsign: &str) -> Option<QueuedCaller> {
        let idx = self.caller_queue.iter().position(|q| q.callsign.eq_ignore_ascii_case(callsign))?;
        Some(self.caller_queue.remove(idx))
    }

    /// Prune given-up callers, then pick the next one to auto-work: FIFO
    /// (first queued) normally, or strongest SNR first when
    /// `strongest_first` -- ties favor the earlier-queued entry, matching
    /// `CallerQueueOrdering.pickNextIndex`. Not wired to any automatic
    /// call site yet (see this feature's plan doc) -- built now since the
    /// UI's own chip ordering needs the identical comparator.
    pub fn dequeue_next(&mut self, strongest_first: bool, now_ms: i64, cycle_ms: i64) -> Option<QueuedCaller> {
        self.prune_given_up_callers(now_ms, cycle_ms);
        if self.caller_queue.is_empty() {
            return None;
        }
        let idx = if strongest_first {
            self.caller_queue
                .iter()
                .enumerate()
                .max_by_key(|(i, q)| (q.snr, std::cmp::Reverse(*i)))
                .map(|(i, _)| i)?
        } else {
            0
        };
        Some(self.caller_queue.remove(idx))
    }

    pub fn clear_caller_queue(&mut self) {
        self.caller_queue.clear();
    }

    /// Shared by total silence and a non-advancing reply (see `advanced`
    /// in `process_rx`): advances whichever give-up counter applies to
    /// the current stage. Checked every time progress didn't happen, not
    /// just on total silence -- Android's `hasReachedMax73Sends` runs
    /// every cycle for exactly this reason: a partner who keeps replying
    /// with something that never confirms (e.g. re-sending his own
    /// report) would otherwise never trip the cap, since presence of
    /// *any* message used to keep resetting the counter.
    /// Returns whether `give_up_target` fired this call -- a watchdog
    /// forced an end to the current target -- so `process_rx`'s callers
    /// can report `QsoOutcome::GaveUpTarget` and unwind excursion-scoped
    /// engine state (see that variant's doc comment).
    fn on_no_progress(&mut self) -> bool {
        match self.stage {
            // Still trying to get a CQ-caller to answer us even once --
            // governed by the narrower cq_answer_limit (capped by, but
            // separate from, no_reply_limit). See `effective_cq_answer_
            // limit`'s own doc comment for the exact composition rule.
            TxStage::Grid => {
                self.no_reply_count = self.no_reply_count.saturating_add(1);
                let limit = effective_cq_answer_limit(self.cq_answer_limit, self.no_reply_limit);
                let gave_up = should_give_up_target(limit, self.no_reply_count);
                log_watchdog_diag(&format!(
                    "{} Grid miss #{} (limit {}) gave_up={}",
                    util::now_unix_ms(), self.no_reply_count, limit, gave_up
                ));
                if gave_up {
                    self.give_up_target();
                    return true;
                }
            }
            // A confirmed, ongoing QSO (he has replied at least once) --
            // governed by the broader no_reply_limit ("TX Watchdog"),
            // floored (effective_no_reply_limit) so 0 can't mean unlimited.
            TxStage::Report | TxStage::RReport => {
                self.no_reply_count = self.no_reply_count.saturating_add(1);
                let limit = effective_no_reply_limit(self.no_reply_limit);
                let gave_up = should_give_up_target(limit, self.no_reply_count);
                log_watchdog_diag(&format!(
                    "{} {:?} miss #{} (limit {}) gave_up={}",
                    util::now_unix_ms(), self.stage, self.no_reply_count, limit, gave_up
                ));
                if gave_up {
                    self.give_up_target();
                    return true;
                }
            }
            TxStage::Rr73 => {
                let unlimited_failsafe =
                    self.max_73_sends == 0 && self.final_ack_sends >= RR73_GIVEUP_CYCLES_FAILSAFE;
                if has_reached_max_73_sends(self.max_73_sends, self.final_ack_sends) || unlimited_failsafe {
                    self.give_up_target();
                    return true;
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
        false
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

    fn build_qso_record(&self, dx: &str) -> QsoRecord {
        QsoRecord {
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
        }
    }

    fn complete(&mut self, dx: &str) -> Option<QsoOutcome> {
        // Already logged -- see `already_logged`'s own doc comment -- by
        // `notify_transmitted` the moment RR73 was confirmed sent. This is
        // just the partner's courtesy reply/repeat catching up; finish the
        // run without a second QsoRecord for the same contact.
        if self.already_logged {
            if self.stage != TxStage::Bye73 {
                self.finish_qso();
            }
            return None;
        }
        let record = self.build_qso_record(dx);

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
            self.already_logged = true;
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

/// TEMPORARY diagnostic helper (see `on_no_progress`'s Grid/Report/RReport
/// branches) -- tracking a live report that the TX/CQ-answer watchdog
/// gives up after far fewer real misses than configured, intermittently
/// (seen as 1, 2, or correctly 3). `eprintln!` was tried first and
/// produced nothing: release Windows builds compile with
/// `windows_subsystem = "windows"` (main.rs), which detaches stdout/
/// stderr entirely, even launched from an existing console -- confirmed
/// live, the operator saw no output at all running from cmd.exe. A plain
/// file is the only reliable way to get this out on Windows without
/// rebuilding as a full console app. Appends; best-effort (a failed write
/// here must never affect real give-up behavior) -- `%TEMP%\ft8af_
/// watchdog_diag.log` on Windows (`std::env::temp_dir()` generally).
/// Remove this whole function and its two call sites once the real cause
/// is found.
pub(crate) fn log_watchdog_diag(line: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("ft8af_watchdog_diag.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// TX Watchdog's floor when left at "unlimited" (0). Removed deliberately,
/// not just defensively (see `DEFAULT_CQ_ANSWER_LIMIT`'s sibling removal):
/// "we do not want this app to become a BOT... there must be a control
/// operator present" -- an indefinitely-retransmitting mid-QSO exchange is
/// exactly the unattended-operation risk that rule exists for, even though
/// it's narrower than bare-CQ's (the partner answered at least once, so
/// *something* is there). More generous than `DEFAULT_CQ_ANSWER_LIMIT`
/// since a partner known to be present deserves more patience than a CQ
/// caller who's never replied even once.
pub const DEFAULT_TX_WATCHDOG_LIMIT: u32 = 5;

/// TX Watchdog ("Stop After" N unanswered attempts), floored to
/// `DEFAULT_TX_WATCHDOG_LIMIT` so a literal 0 can never mean "unlimited" --
/// see that constant's doc comment. Ported in spirit from Android's
/// `FT8TransmitSignal.shouldGiveUpTarget` -- desktop's stage model differs
/// enough (7 named `TxStage`s vs. Android's 6 numeric `functionOrder`s)
/// that this re-derives the same "silence since the target last replied"
/// signal natively via `no_reply_count` rather than porting Android's
/// order-number arithmetic line for line.
pub fn effective_no_reply_limit(no_reply_limit: u32) -> u32 {
    if no_reply_limit == 0 { DEFAULT_TX_WATCHDOG_LIMIT } else { no_reply_limit }
}

/// The actual give-up comparison, deliberately still pure/unfloored (a
/// literal `no_reply_limit=0` genuinely means "never give up" to THIS
/// function) -- every real call site floors its own `no_reply_limit`
/// first, via `effective_no_reply_limit` or `effective_cq_answer_limit`,
/// so 0 never reaches here from production code.
pub fn should_give_up_target(no_reply_limit: u32, no_reply_count: u32) -> bool {
    no_reply_limit > 0 && no_reply_count >= no_reply_limit
}

/// `CQ Answer Attempts`' floor when nothing valid is configured --
/// previously 0 meant "Same as TX Watchdog" (defer entirely to
/// `no_reply_limit`), but that deferred path kept giving up after a single
/// real attempt on a live Windows run despite every test here (including a
/// full `Engine::handle_decoded` simulation of the exact scenario)
/// confirming the math was correct in isolation -- the operator's own
/// explicit fix, each time, was to set this to a real number instead of
/// leaving it deferred. Rather than keep chasing an elusive, seemingly
/// Windows-only defer-path bug, the option itself is removed: this is now
/// only a defensive floor for a stray 0 that slips through some other path
/// (the UI enforces a minimum of 1 on save; `Engine::new` migrates a
/// legacy-persisted 0 to this same value and writes it back).
pub const DEFAULT_CQ_ANSWER_LIMIT: u32 = 3;

/// The real cap on `TxStage::Grid` retries (answering a CQ that's never
/// replied even once): never allowed to exceed `no_reply_limit` ("TX
/// Watchdog") -- both floored first (`DEFAULT_CQ_ANSWER_LIMIT`,
/// `effective_no_reply_limit`), so neither side can mean "unlimited" and
/// the clamp is always a real, finite comparison.
pub fn effective_cq_answer_limit(cq_answer_limit: u32, no_reply_limit: u32) -> u32 {
    let limit = if cq_answer_limit == 0 { DEFAULT_CQ_ANSWER_LIMIT } else { cq_answer_limit };
    limit.min(effective_no_reply_limit(no_reply_limit))
}

/// CQ Watchdog's floor when left at "unlimited" (0). Removed deliberately:
/// "we do not want this app to become a BOT... I've seen stations calling
/// CQ for hours at night... there must be a control operator present" --
/// this is the exact scenario that rule targets (bare CQ with nobody
/// answering at all, the "robot" case this constant's old doc comment
/// explicitly invited). A long stretch calling CQ is normal and expected;
/// running all night unattended is not -- this cap is generous enough for
/// the former while ruling out the latter.
pub const DEFAULT_CQ_WATCHDOG_LIMIT: u32 = 40;

/// CQ Watchdog cap, floored to `DEFAULT_CQ_WATCHDOG_LIMIT` so a literal 0
/// can never mean "unlimited" -- see that constant's doc comment for why.
/// See `QsoEngine::cq_no_reply_limit`'s doc comment for why this is a
/// separate setting/counter from `should_give_up_target`'s.
pub fn should_give_up_cq(cq_no_reply_limit: u32, cq_no_reply_count: u32) -> bool {
    let limit = if cq_no_reply_limit == 0 { DEFAULT_CQ_WATCHDOG_LIMIT } else { cq_no_reply_limit };
    cq_no_reply_count >= limit
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

/// Whether this slot's decodes show `target` actively exchanging with a
/// DIFFERENT station -- a real signal, not just silence, that he's moved
/// on and further calls are futile. Real UX requirement, stated live:
/// "I'm answering station xyz's CQ but he answers someone else not my
/// station. Stop calling him because he's in QSO with someone else...
/// if his next transmission is not 'CQ his_call his_grid' or 'my_call
/// his_call my_signal_report' I stop trying to call him." Detected
/// directly from message content, not a reply-count/timeout -- any
/// message FROM the target that is neither a CQ (he just hasn't heard
/// anyone yet) nor addressed to us (continuing our own exchange) means
/// he's talking to someone else specifically. A single such decode is
/// enough to trigger this (matching the operator's own stated rule
/// exactly) -- a busted/misheard callsign could in principle cause a
/// false positive, same risk any single FT8 decode carries elsewhere in
/// this engine.
pub fn target_working_someone_else(messages: &[DecodedMessage], target: &str, my_call: &str) -> bool {
    messages.iter().any(|m| {
        m.call_from.eq_ignore_ascii_case(target)
            && !crate::callsign::is_cq_callsign(&m.call_to)
            && !crate::callsign::is_my_callsign(&m.call_to, my_call)
    })
}

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

        // DX sends RR73 -> we queue the courtesy 73 to transmit. The decoder
        // copies the grid-shaped "RR73" into the grid field, so pass it here
        // as it actually arrives live (grid == "RR73"), not grid == "".
        // Real bug, ported from a fix already made on the Android side: this
        // must NOT log yet -- only once notify_transmitted confirms the 73
        // was actually sent (see that method's own doc comment). Logging
        // here, before the 73 has gone out, risked a logged-but-never-sent
        // QSO if that transmission then failed.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        assert!(out.is_none(), "must not log until the courtesy 73 is actually sent");
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);

        // The scheduler actually transmits it -- THIS logs the QSO.
        match e.notify_transmitted() {
            Some(QsoOutcome::Completed(rec)) => {
                assert_eq!(rec.call, "K1ABC");
                assert_eq!(rec.gridsquare, "FN42");
                assert_eq!(rec.rst_sent, "-08");
                assert_eq!(rec.rst_rcvd, "-12");
            }
            _ => panic!("expected the QSO to log the instant the courtesy 73 was confirmed sent"),
        }
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

        // DX sends RR73 -> the courtesy 73 is queued (not yet logged -- that
        // only happens once notify_transmitted confirms it was actually
        // sent). As live, the grid-shaped "RR73" also lands in the
        // decoder's grid field.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        assert!(out.is_none());
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);

        // The 73 did NOT go out this slot (no notify_transmitted) -- so it's
        // not logged yet either. The next slot must keep the 73 queued
        // rather than returning to CQ, and must not log the QSO a second
        // time once it finally does go out.
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);
        // Still pending after another silent slot.
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"));
        assert_eq!(e.status().stage, TxStage::Bye73);

        // Once the scheduler reports the 73 actually transmitted, THIS logs
        // the QSO (not any of the silent slots above), and the next slot
        // wraps it up and returns to CQ.
        assert!(matches!(e.notify_transmitted(), Some(QsoOutcome::Completed(_))));
        assert!(e.process_rx(&[]).is_none(), "must not log a second time");
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn caller_logs_the_instant_rr73_is_sent_even_if_no_73_ever_comes_back() {
        // Real bug, found live: "I sent my RR73 but never received his 73.
        // I should have logged the QSO but did not." Standard FT8 practice
        // (and Android's own order-4 "RR73 sent" save): sending RR73
        // confirms both signal reports were exchanged -- the contact is
        // already valid and loggable right then. A reply "73" is pure
        // courtesy, never required. Before this fix, the CALLER's side (the
        // one who calls CQ and eventually sends RR73, as opposed to the
        // answerer's side already covered by answerer_full_sequence_logs_qso)
        // had no logging path at all except receiving a reply back while in
        // TxStage::Rr73 -- dead silence meant the QSO was simply never built,
        // and give_up_target (after Max 73 Sends) discarded it with nothing
        // ever logged.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.start_cq();
        // K1ABC answers our CQ with his grid.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "FN42", "FN42", -5)]);
        assert!(out.is_none());
        assert_eq!(e.status().stage, TxStage::Report);
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ -05"));

        // He sends his R+report -- Report's own match arm goes straight to
        // sending RR73 (no separate RReport-sending stage on this side).
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "R-08", "", -8)]);
        assert!(out.is_none());
        assert_eq!(e.status().stage, TxStage::Rr73);
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ RR73"));

        // The scheduler confirms RR73 actually went out -- this must log the
        // QSO immediately, with no reply ever required.
        let out = e.notify_transmitted();
        match out {
            Some(QsoOutcome::Completed(rec)) => {
                assert_eq!(rec.call, "K1ABC");
                assert_eq!(rec.gridsquare, "FN42");
            }
            _ => panic!("expected the QSO to log the instant RR73 was confirmed sent"),
        }
        assert_eq!(e.status().stage, TxStage::Rr73, "still waiting/retrying for a courtesy 73, unaffected by logging");

        // Dead silence from here on (the user's exact scenario) -- still
        // logged once, no duplicate, and the run eventually gives up
        // cleanly without ever trying to log a second time.
        for _ in 0..10 {
            assert!(e.process_rx(&[]).is_none());
        }
    }

    #[test]
    fn a_late_courtesy_73_does_not_log_the_qso_a_second_time() {
        // The flip side of the fix above: if the partner's courtesy 73 DOES
        // eventually arrive after RR73 was already confirmed sent, it must
        // just finish the run (back to CQ / idle), not build a duplicate
        // QsoRecord for the same contact.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.start_cq();
        e.process_rx(&[msg("K0XYZ", "K1ABC", "FN42", "FN42", -5)]);
        e.process_rx(&[msg("K0XYZ", "K1ABC", "R-08", "", -8)]);
        assert!(matches!(e.notify_transmitted(), Some(QsoOutcome::Completed(_))));

        // His courtesy 73 finally arrives.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "73", "", -8)]);
        assert!(out.is_none(), "must not log a second record for the same contact");
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn manually_answering_someone_else_mid_retry_does_not_relog_the_first_contact() {
        // Real scenario, confirmed live: the operator can see BOTH the
        // first station's (K1ABC) repeated report AND a second station
        // (K5ZZZ) calling, once the panel has reverted after K1ABC's QSO
        // was already logged (RR73 sent). Manually choosing to answer
        // K5ZZZ instead of continuing to retry with K1ABC ("not the
        // gentleman thing to do," per the operator, but a real, supported
        // choice) must NOT re-log K1ABC -- his record is already in,
        // `answer()`'s own `reset_qso()` simply abandons the retry/dormant
        // state cleanly, and K5ZZZ gets his own fresh, independently
        // logged QSO.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.start_cq();
        e.process_rx(&[msg("K0XYZ", "K1ABC", "FN42", "FN42", -5)]);
        e.process_rx(&[msg("K0XYZ", "K1ABC", "R-08", "", -8)]);
        assert!(matches!(e.notify_transmitted(), Some(QsoOutcome::Completed(_))), "K1ABC logs once, on RR73 sent");
        assert_eq!(e.status().stage, TxStage::Rr73, "still quietly retrying with K1ABC");

        // Operator manually answers K5ZZZ instead, abandoning K1ABC's retry.
        e.answer(&msg("CQ", "K5ZZZ", "EM12", "EM12", -3));
        assert_eq!(e.status().target.as_deref(), Some("K5ZZZ"));
        assert_eq!(e.status().stage, TxStage::Grid);

        // Even if K1ABC's repeat somehow still arrives right after the
        // switch, it can no longer match the (now different) target --
        // no re-log, no interference with the new exchange.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "R-08", "", -8)]);
        assert!(out.is_none(), "K1ABC's repeat can't match K5ZZZ's QSO -- not mine, counts as silence, no re-log");
        assert_eq!(e.status().target.as_deref(), Some("K5ZZZ"), "the switch to K5ZZZ is unaffected");

        // K5ZZZ's own exchange logs independently, exactly once -- we
        // answered HIS CQ, so this is the answerer's path: he sends a
        // plain report, we send our R-report, his RR73 queues our courtesy
        // 73, and logging happens once THAT is confirmed sent.
        e.process_rx(&[msg("K0XYZ", "K5ZZZ", "-10", "", -8)]);
        assert_eq!(e.status().stage, TxStage::RReport);
        assert!(e.process_rx(&[msg("K0XYZ", "K5ZZZ", "RR73", "RR73", -8)]).is_none());
        assert_eq!(e.status().stage, TxStage::Bye73);
        match e.notify_transmitted() {
            Some(QsoOutcome::Completed(rec)) => assert_eq!(rec.call, "K5ZZZ"),
            other => panic!("expected K5ZZZ's own QSO to log the instant his courtesy 73 was confirmed sent, got {other:?}"),
        }
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

        // DX signs off with RR73 exactly as the decoder emits it live. Must
        // advance (not hang retransmitting the R-report) -- logging itself
        // now waits for notify_transmitted (see its own doc comment), so
        // this only checks the advance, not an immediate QsoOutcome.
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        assert!(out.is_none());
        assert_eq!(e.tx_message(), Some("K1ABC K0XYZ 73"), "RR73 sign-off must advance to the courtesy 73, not hang");
        assert_eq!(e.status().stage, TxStage::Bye73);

        match e.notify_transmitted() {
            Some(QsoOutcome::Completed(rec)) => {
                assert_eq!(rec.call, "K1ABC");
                assert_eq!(rec.gridsquare, "FN42");
                assert_eq!(rec.rst_sent, "-08");
                assert_eq!(rec.rst_rcvd, "-12");
            }
            _ => panic!("expected the QSO to log the instant the courtesy 73 was confirmed sent"),
        }
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
    fn effective_cq_answer_limit_pure_cases() {
        // A literal 0 (the removed "Same as TX Watchdog" option) no longer
        // defers -- it's defensively treated as DEFAULT_CQ_ANSWER_LIMIT,
        // itself still clamped down to a finite TX Watchdog.
        assert_eq!(effective_cq_answer_limit(0, 0), DEFAULT_CQ_ANSWER_LIMIT);
        assert_eq!(effective_cq_answer_limit(0, 2), 2);
        assert_eq!(effective_cq_answer_limit(0, 5), DEFAULT_CQ_ANSWER_LIMIT);

        // TX Watchdog left at its own "unlimited" (0) -- also removed
        // (see DEFAULT_TX_WATCHDOG_LIMIT's doc comment), so this clamps to
        // ITS floor too, not "no ceiling at all" like before.
        assert_eq!(effective_cq_answer_limit(1, 0), 1); // below the floor either way
        assert_eq!(effective_cq_answer_limit(10, 0), DEFAULT_TX_WATCHDOG_LIMIT); // clamped to the floor, not 10

        // Both finite -- never exceeds TX Watchdog's own value.
        assert_eq!(effective_cq_answer_limit(1, 5), 1); // "Personally I'd set it to one"
        assert_eq!(effective_cq_answer_limit(5, 1), 1); // clamped down, not up
        assert_eq!(effective_cq_answer_limit(3, 3), 3); // equal is fine
    }

    #[test]
    fn target_working_someone_else_pure_cases() {
        // Real UX requirement, stated live: detect he's moved on to a
        // different station directly from message content, not a
        // reply-count/timeout.
        let my_call = "K0XYZ";
        let target = "K1ABC";

        // His report to a DIFFERENT station -- he's working someone else.
        assert!(target_working_someone_else(
            &[msg("W1AW", "K1ABC", "-05", "", -5)],
            target,
            my_call,
        ));

        // His own repeated CQ -- he hasn't heard anyone yet, not "someone else".
        assert!(!target_working_someone_else(
            &[msg("CQ", "K1ABC", "FN42", "FN42", -5)],
            target,
            my_call,
        ));

        // A message addressed to US -- continuing our own exchange.
        assert!(!target_working_someone_else(
            &[msg("K0XYZ", "K1ABC", "-05", "", -5)],
            target,
            my_call,
        ));

        // Traffic from someone other than the target entirely -- irrelevant.
        assert!(!target_working_someone_else(
            &[msg("W1AW", "N2JFD", "-05", "", -5)],
            target,
            my_call,
        ));

        // Empty slot -- nothing to detect.
        assert!(!target_working_someone_else(&[], target, my_call));
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
        // 0 ("unlimited"/robot operation) is removed -- floors to
        // DEFAULT_CQ_WATCHDOG_LIMIT instead of never giving up (see that
        // constant's doc comment: "we do not want this app to become a
        // BOT... there must be a control operator present").
        assert!(!should_give_up_cq(0, DEFAULT_CQ_WATCHDOG_LIMIT - 1));
        assert!(should_give_up_cq(0, DEFAULT_CQ_WATCHDOG_LIMIT));
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
    fn cq_watchdog_left_at_default_still_gives_up_eventually() {
        // Was "cq_watchdog_disabled_calls_cq_forever" -- 0 used to mean
        // truly unlimited (the removed "robot operation" default). "we do
        // not want this app to become a BOT... there must be a control
        // operator present" -- a bare, unanswered CQ now always gives up
        // by DEFAULT_CQ_WATCHDOG_LIMIT, generous enough for a long stretch
        // of calling but never indefinite.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.cq_no_reply_limit = 0;
        e.start_cq();
        for _ in 0..(DEFAULT_CQ_WATCHDOG_LIMIT as usize - 1) {
            assert!(e.process_rx(&[]).is_none());
        }
        assert!(e.status().active, "must not give up before the default cap");
        assert_eq!(e.status().stage, TxStage::Cq);

        assert!(e.process_rx(&[]).is_none());
        assert!(!e.status().active, "must give up at the default cap, never run forever");
        assert_eq!(e.status().stage, TxStage::Idle);
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
        e.cq_no_reply_limit = 0; // floors to DEFAULT_CQ_WATCHDOG_LIMIT (40) -- well short of giving up within this test's 10 cycles
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
        // The 2nd silent slot hits the limit and gives up -> back to CQ,
        // signaling the engine so it can unwind excursion-scoped state
        // (e.g. a pending `preferred_tx_parity` restore).
        assert!(matches!(e.process_rx(&[]), Some(QsoOutcome::GaveUpTarget)));
        assert_eq!(e.status().stage, TxStage::Cq);
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
    }

    #[test]
    fn cq_answer_limit_gives_up_faster_than_tx_watchdog_while_still_answering_his_cq() {
        // Real UX requirement, stated live: "Personally I'd set it to
        // 'one'" -- a CQ that's never replied even once should give up
        // after just ONE silent retry, well before the broader TX
        // Watchdog's own (higher) limit would fire.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 5; // the broader mid-QSO cap stays generous
        e.cq_answer_limit = 1; // but answering a never-replying CQ gives up fast
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);

        // The very first silent slot already hits the (narrower) cap.
        assert!(matches!(e.process_rx(&[]), Some(QsoOutcome::GaveUpTarget)));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn cq_answer_limit_never_exceeds_tx_watchdog_even_if_configured_higher() {
        // "the value of x has to be less than or equal to the value of TX
        // Watchdog" -- a CQ-answer cap configured ABOVE TX Watchdog is
        // clamped down to it, not honored as typed.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 2;
        e.cq_answer_limit = 10; // configured far higher than TX Watchdog
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);

        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.status().stage, TxStage::Grid, "1st silent slot -- not given up yet");
        // Gives up at TX Watchdog's own limit (2), not the configured 10.
        assert!(matches!(e.process_rx(&[]), Some(QsoOutcome::GaveUpTarget)));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn cq_answer_limit_provides_a_real_cap_even_when_tx_watchdog_is_unlimited() {
        // The actual safety net the operator asked for: "want to avoid
        // [a case like] the TX Watchdog being set to unlimited so I keep
        // calling him forever."
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 0; // TX Watchdog unlimited
        e.cq_answer_limit = 1;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);

        assert!(matches!(e.process_rx(&[]), Some(QsoOutcome::GaveUpTarget)));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn cq_answer_limit_only_governs_grid_stage_not_a_confirmed_qso() {
        // Once he's replied at least once (we're past Grid), the broader
        // no_reply_limit ("TX Watchdog") governs -- not cq_answer_limit.
        // The operator's own framing: "while I'm in a QSO I'd want the TX
        // Watchdog setting to control how many times I will retransmit a
        // message."
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 3;
        e.cq_answer_limit = 1; // would give up immediately if this applied here
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        // He replies with a report -> advances past Grid into RReport (we
        // now send OUR report back, waiting for his RR73/RRR).
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        assert_eq!(e.status().stage, TxStage::RReport);

        // Two silent slots now -- still under no_reply_limit=3, and NOT
        // cut short by cq_answer_limit=1 (which no longer applies here).
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.status().stage, TxStage::RReport);
        assert!(e.process_rx(&[]).is_none());
        assert_eq!(e.status().stage, TxStage::RReport, "cq_answer_limit must not apply mid-QSO");
    }

    #[test]
    fn cq_answer_limit_equal_to_tx_watchdog_takes_three_silent_slots() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 3;
        e.cq_answer_limit = 3;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);

        assert!(e.process_rx(&[]).is_none(), "1st silent slot -- must not give up yet");
        assert_eq!(e.status().stage, TxStage::Grid);
        assert!(e.process_rx(&[]).is_none(), "2nd silent slot -- must not give up yet");
        assert_eq!(e.status().stage, TxStage::Grid);
        assert!(matches!(e.process_rx(&[]), Some(QsoOutcome::GaveUpTarget)), "3rd silent slot -- now give up");
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn gives_up_immediately_when_the_target_works_someone_else() {
        // Real UX requirement, stated live: "I'm answering station xyz's
        // CQ but he answers someone else not my station. Stop calling him
        // because he's in QSO with someone else." No reply limit needed --
        // this must fire on the very first decode showing it, not wait out
        // a multi-cycle watchdog.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 0; // unlimited -- proves this isn't the watchdog firing
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);

        // He's sending a report to a different station entirely.
        let outcome = e.process_rx(&[msg("W1AW", "K1ABC", "-05", "", -5)]);
        assert!(matches!(outcome, Some(QsoOutcome::GaveUpTarget)));
        assert_eq!(e.status().stage, TxStage::Cq);
        assert_eq!(e.tx_message(), Some("CQ K0XYZ EN37"));
    }

    #[test]
    fn a_repeated_cq_from_the_target_does_not_trigger_the_working_someone_else_give_up() {
        // The dormant/no-progress path, not this one -- he hasn't heard
        // anyone yet, which is a real difference from working someone else.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 0;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);

        assert!(e.process_rx(&[msg("CQ", "K1ABC", "FN42", "FN42", -5)]).is_none());
        assert_eq!(e.status().stage, TxStage::Grid, "still waiting, not given up");
    }

    #[test]
    fn tx_watchdog_disabled_but_cq_answer_limit_still_caps_grid_stage_retries() {
        // Grid stage (answering a CQ that's never replied) now always has a
        // real cap -- a fresh/unconfigured engine's cq_answer_limit (still
        // 0, its struct default) is floored to DEFAULT_CQ_ANSWER_LIMIT by
        // effective_cq_answer_limit, even with TX Watchdog left unlimited.
        // This used to be "never gives up" (the removed "Same as TX
        // Watchdog" (0) option deferred entirely to no_reply_limit, which
        // defers to nothing when that's also 0) -- deliberately changed:
        // nobody ever asked for *unlimited* retries chasing a CQ caller who
        // never replies even once, and the old behavior left a fresh
        // install with that exact trap.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 0;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        for _ in 0..(DEFAULT_CQ_ANSWER_LIMIT as usize - 1) {
            assert!(e.process_rx(&[]).is_none(), "must not give up before the default cap");
        }
        assert_eq!(e.status().stage, TxStage::Grid);
        assert!(matches!(e.process_rx(&[]), Some(QsoOutcome::GaveUpTarget)));
        assert_eq!(e.status().stage, TxStage::Cq);
    }

    #[test]
    fn tx_watchdog_and_cq_answer_limit_both_explicitly_unlimited_is_not_possible() {
        // Confirms there is no remaining way to make Grid-stage retries
        // truly unlimited: even the maximum nonsense input (0 for both)
        // still gives up at the default cap, never runs forever.
        assert_eq!(effective_cq_answer_limit(0, 0), DEFAULT_CQ_ANSWER_LIMIT);
        assert!(should_give_up_target(effective_cq_answer_limit(0, 0), DEFAULT_CQ_ANSWER_LIMIT));
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
    fn a_partner_repeat_resets_the_watchdog_instead_of_counting_against_it() {
        // Real UX requirement, stated live: "the [TX Watchdog] counter is
        // incremented each time i send a message but that is assuming no
        // response from the other station. If there happens to be a
        // response to a subsequent transmission the TX Watchdog counter
        // should be reinitialized... If he responds to me before the TX
        // Watchdog setting is reached I should get another 1 thru TX
        // Watchdog setting opportunity to reply to him." A live partner
        // repeating himself (not copying our last reply) is a
        // fundamentally different, much less worrying situation than
        // total silence -- reverted from this test's own prior assertion
        // that a repeat counts as a failed attempt same as silence.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 2;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        assert_eq!(e.status().stage, TxStage::Grid);
        // He keeps re-sending his grid (what he already sent before we
        // even answered) instead of a report -- never advances Grid, but
        // he's clearly still there every single time.
        for _ in 0..5 {
            e.process_rx(&[msg("K0XYZ", "K1ABC", "", "FN42", -8)]);
            assert_eq!(e.status().stage, TxStage::Grid, "a live repeat must never count against the watchdog");
        }
        // True silence now DOES count, same as always.
        e.process_rx(&[]);
        assert_eq!(e.status().stage, TxStage::Grid, "1st real silent slot");
        e.process_rx(&[]);
        assert_eq!(e.status().stage, TxStage::Cq, "2nd real silent slot hits the limit");
    }

    #[test]
    fn a_genuine_repeat_after_silence_grants_a_fresh_watchdog_budget() {
        // The operator's own exact worked example: "TX Watchdog set to 3.
        // I call a station, he answers me and the TX Watchdog has gotten
        // incremented, I respond to him but get no response from his
        // station, TX Watchdog get incremented, in the next slot I get a
        // response from him, and reply, tx watchdog gets incremented
        // again and is at 3 so I give up... that should not happen."
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.no_reply_limit = 3;
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5)); // -> Grid
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]); // his report -> RReport
        assert_eq!(e.status().stage, TxStage::RReport);

        e.process_rx(&[]); // 1 real silent slot
        assert_eq!(e.status().stage, TxStage::RReport);
        // He's still there, just repeating his report (hasn't copied our
        // reply) -- must NOT be the 2nd strike toward a 3-strike give up.
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        assert_eq!(e.status().stage, TxStage::RReport, "a live repeat resets, it's not strike 2");

        // Full fresh budget from here -- 2 more silent slots must NOT give up.
        e.process_rx(&[]);
        assert_eq!(e.status().stage, TxStage::RReport, "1st silent slot of the fresh budget");
        e.process_rx(&[]);
        assert_eq!(e.status().stage, TxStage::RReport, "2nd silent slot of the fresh budget");
        // Only the 3rd silent slot in a row (never reset again) gives up.
        e.process_rx(&[]);
        assert_eq!(e.status().stage, TxStage::Cq, "3rd consecutive real silent slot");
    }

    #[test]
    fn dormant_repeat_answers_without_logging_a_duplicate_qso() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.auto_return_to_cq = false; // Android's precondition: TX off after a logged QSO
        e.band = "20m".to_string();
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]); // -> RReport, R-08 sent
        let out = e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]); // queues 73, not yet logged
        assert!(out.is_none());
        assert!(matches!(e.notify_transmitted(), Some(QsoOutcome::Completed(_)))); // 73 sent -> logs now
        assert!(e.process_rx(&[]).is_none()); // -> TX stops (auto_return_to_cq=false)
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

    // --- Caller queue (pileup-while-busy) -----------------------------

    const CYCLE_MS: i64 = 15_000;

    #[test]
    fn a_third_station_calling_mid_qso_is_queued() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 3)], 1_000, CYCLE_MS);
        assert_eq!(e.status().caller_queue.len(), 1);
        assert_eq!(e.status().caller_queue[0].callsign, "IT9ABC");
        assert_eq!(e.status().caller_queue[0].snr, 3);
    }

    #[test]
    fn the_current_partner_and_bare_73s_are_never_queued() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(
            &[
                msg("K0XYZ", "K1ABC", "-12", "", -8), // our own partner
                msg("K0XYZ", "IT9ABC", "73", "", 3),  // a bare courtesy 73
            ],
            1_000,
            CYCLE_MS,
        );
        assert!(e.status().caller_queue.is_empty());
    }

    #[test]
    fn offer_callers_does_nothing_when_idle_or_untargeted() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 3)], 1_000, CYCLE_MS);
        assert!(e.status().caller_queue.is_empty());
    }

    #[test]
    fn calling_again_refreshes_snr_without_duplicating_the_entry() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 3)], 1_000, CYCLE_MS);
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 9)], 2_000, CYCLE_MS);
        assert_eq!(e.status().caller_queue.len(), 1);
        assert_eq!(e.status().caller_queue[0].snr, 9);
    }

    #[test]
    fn a_caller_who_goes_quiet_for_a_full_cycle_is_pruned() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 3)], 0, CYCLE_MS);
        assert_eq!(e.status().caller_queue.len(), 1);
        // Exactly one full cycle (2 slots) of silence -- boundary inclusive,
        // matching Android's idleBoundary_isInclusive -- still present.
        e.offer_callers(&[], 2 * CYCLE_MS, CYCLE_MS);
        assert_eq!(e.status().caller_queue.len(), 1);
        // One slot further -- now stale, pruned.
        e.offer_callers(&[], 2 * CYCLE_MS + 1, CYCLE_MS);
        assert!(e.status().caller_queue.is_empty());
    }

    #[test]
    fn caller_queue_caps_at_ten() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        let decodes: Vec<_> = (0..12)
            .map(|i| msg("K0XYZ", &format!("IT9A{i:02}"), "JN45", "JN45", i))
            .collect();
        e.offer_callers(&decodes, 1_000, CYCLE_MS);
        assert_eq!(e.status().caller_queue.len(), MAX_CALLER_QUEUE);
    }

    #[test]
    fn dequeue_specific_ignores_staleness_the_operator_explicitly_tapped() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 3)], 0, CYCLE_MS);
        // Long past the prune window -- dequeue_specific must still honor
        // the operator's own explicit tap.
        let caller = e.dequeue_specific("IT9ABC");
        assert_eq!(caller.unwrap().callsign, "IT9ABC");
        assert!(e.status().caller_queue.is_empty());
        assert!(e.dequeue_specific("IT9ABC").is_none(), "already removed");
    }

    #[test]
    fn dequeue_next_fifo_by_default() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "FIRST", "JN45", "JN45", -5)], 0, CYCLE_MS);
        e.offer_callers(&[msg("K0XYZ", "SECOND", "JN45", "JN45", 10)], 100, CYCLE_MS);
        let next = e.dequeue_next(false, 200, CYCLE_MS).unwrap();
        assert_eq!(next.callsign, "FIRST", "FIFO: earliest-queued wins even with a weaker signal");
    }

    #[test]
    fn dequeue_next_strongest_first_breaks_ties_by_earliest_queued() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "FIRST", "JN45", "JN45", -5)], 0, CYCLE_MS);
        e.offer_callers(&[msg("K0XYZ", "SECOND", "JN45", "JN45", 10)], 100, CYCLE_MS);
        let next = e.dequeue_next(true, 200, CYCLE_MS).unwrap();
        assert_eq!(next.callsign, "SECOND", "strongest SNR wins when enabled");

        // Tie case: equal SNR favors the earlier-queued entry.
        let mut e2 = QsoEngine::new("K0XYZ", "EN37");
        e2.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e2.offer_callers(&[msg("K0XYZ", "FIRST", "JN45", "JN45", 5)], 0, CYCLE_MS);
        e2.offer_callers(&[msg("K0XYZ", "SECOND", "JN45", "JN45", 5)], 100, CYCLE_MS);
        let next2 = e2.dequeue_next(true, 200, CYCLE_MS).unwrap();
        assert_eq!(next2.callsign, "FIRST", "equal SNR: earliest-queued wins the tie");
    }

    #[test]
    fn stop_and_start_cq_clear_the_caller_queue_but_answer_does_not() {
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 3)], 0, CYCLE_MS);
        assert_eq!(e.status().caller_queue.len(), 1);

        // Switching target via answer() (e.g. tapping a DIFFERENT decode,
        // not this queued caller) must leave the queue intact for later.
        e.answer(&msg("CQ", "W1AW", "FN31", "FN31", -2));
        assert_eq!(e.status().caller_queue.len(), 1, "answer() must not clear other queued callers");

        e.stop();
        assert!(e.status().caller_queue.is_empty(), "stop() clears the queue");

        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.offer_callers(&[msg("K0XYZ", "IT9ABC", "JN45", "JN45", 3)], 0, CYCLE_MS);
        e.start_cq();
        assert!(e.status().caller_queue.is_empty(), "start_cq() clears the queue");
    }

    #[test]
    fn answer_caller_with_a_grid_shaped_message_replies_with_a_report() {
        // Live bug: this used to send our own grid (answer()'s behavior),
        // wrong here since his message already gave us his grid.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer_caller(&msg("K0XYZ", "IT9XYZ", "JN54", "JN54", 7));
        assert_eq!(e.status().target.as_deref(), Some("IT9XYZ"));
        assert_eq!(e.status().stage, TxStage::Report);
        assert_eq!(e.tx_message(), Some("IT9XYZ K0XYZ +07"));
    }

    #[test]
    fn answer_caller_with_a_report_shaped_message_replies_with_an_r_report() {
        // His stored message was itself already a signal report to us
        // (further along than a fresh grid-response) -- reply with R+report,
        // not restart at Grid.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer_caller(&msg("K0XYZ", "IT9XYZ", "-12", "", 7));
        assert_eq!(e.status().target.as_deref(), Some("IT9XYZ"));
        assert_eq!(e.status().stage, TxStage::RReport);
        assert_eq!(e.tx_message(), Some("IT9XYZ K0XYZ R+07"));
    }

    #[test]
    fn answer_caller_falls_back_to_a_grid_reply_for_an_unrecognized_content_shape() {
        // A bare directed message with neither a grid nor a parseable
        // report (e.g. his own RR73/73 from an unrelated, stale exchange)
        // -- the safe baseline, same as plain answer()'s always-Grid reply.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer_caller(&msg("K0XYZ", "IT9XYZ", "RR73", "RR73", 7));
        assert_eq!(e.status().target.as_deref(), Some("IT9XYZ"));
        assert_eq!(e.status().stage, TxStage::Grid);
        assert_eq!(e.tx_message(), Some("IT9XYZ K0XYZ EN37"));
    }

    #[test]
    fn exact_reported_bug_a_third_station_calls_during_the_courtesy_73_tail() {
        // Reproduces the live bug report: operator answered K1ABC's CQ,
        // reached the courtesy-73 retry stage (already logged -- see
        // notify_transmitted's own doc comment), and a different station
        // (modeled here as an Italian prefix) called directly. Before this
        // feature existed, that third station was invisible to the engine
        // entirely during Bye73 (process_rx's own early return). Mirrors
        // Android's CallerQueueLivenessTest.completion_worksTheCallerStillCalling.
        let mut e = QsoEngine::new("K0XYZ", "EN37");
        e.answer(&msg("CQ", "K1ABC", "FN42", "FN42", -5));
        e.process_rx(&[msg("K0XYZ", "K1ABC", "-12", "", -8)]);
        e.process_rx(&[msg("K0XYZ", "K1ABC", "RR73", "RR73", -8)]);
        e.notify_transmitted(); // logs K1ABC, enters Bye73 repeat territory
        assert_eq!(e.status().stage, TxStage::Bye73);
        assert!(e.status().target.as_deref() == Some("K1ABC"));

        // IT9XYZ calls directly while we're repeating the courtesy 73.
        e.offer_callers(&[msg("K0XYZ", "IT9XYZ", "JN54", "JN54", 7)], 5_000, CYCLE_MS);
        assert_eq!(e.status().caller_queue.len(), 1);
        assert_eq!(e.status().caller_queue[0].callsign, "IT9XYZ");

        // Operator taps the queued caller: dequeue, then answer_caller --
        // K1ABC stays cleanly logged (already_logged, untouched), IT9XYZ
        // becomes the new target with a REPORT reply (not another grid
        // exchange -- his own captured message already gave us his grid,
        // see answer_caller's own doc comment for the live bug this part
        // fixes).
        let caller = e.dequeue_specific("IT9XYZ").unwrap();
        e.answer_caller(&msg(&e.my_call.clone(), &caller.callsign, &caller.extra, &caller.grid, caller.snr));
        assert_eq!(e.status().target.as_deref(), Some("IT9XYZ"));
        assert_eq!(e.status().stage, TxStage::Report);
        assert_eq!(e.tx_message(), Some("IT9XYZ K0XYZ +07"));
    }
}
