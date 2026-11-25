//! Safe FT8 decoder wrapper replicating the `decoder_state` lifecycle from
//! `ft8af_glue/ft8_decoder.cpp`:
//!   new -> feed_slot -> find_sync -> decode_all -> (drop frees the monitor).
//!
//! Owns the C `monitor_t` (freed in `Drop`), a candidate array (capped at
//! `MAX_CANDIDATES` below), and a per-decoder callsign hash table for
//! resolving hashed compound calls.

use super::ffi;
use super::hashtable::{HashGuard, HashTable};
use parking_lot::Mutex;
use std::os::raw::c_char;

// KEEP IN SYNC (manually): ft8af/app/src/main/cpp/ft8af_glue/decode_params.h
// holds the canonical, benchmark-justified values (see decode_bench.c).
const MAX_CANDIDATES: usize = 256;
const MIN_SCORE_FAST: i32 = 10;
const MIN_SCORE_DEEP: i32 = 5;
const LDPC_ITERS_NORMAL: i32 = 20;
const LDPC_ITERS_DEEP: i32 = 30;

/// Below this many candidates, fanning `decode_all`'s loop out across worker
/// threads costs more (thread spawn/join) than it saves -- run inline.
const MIN_CANDIDATES_TO_PARALLELIZE: usize = 8;

/// Worker threads `decode_all` may use, from a configured core-reservation.
/// `available_parallelism()`'s failure case (platform can't tell) falls back
/// to 1, same as its own documented caller guidance. Never returns 0: a
/// reservation that eats every detected core still leaves one worker, so
/// decoding is never accidentally disabled by this setting.
pub fn decode_worker_count(reserved: u32) -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .saturating_sub(reserved as usize)
        .max(1)
}

/// A single decoded FT8 message with geometry + signal metrics.
#[derive(Debug, Clone, Default)]
pub struct DecodedMessage {
    pub call_to: String,
    pub call_from: String,
    pub extra: String,
    pub grid: String,
    pub raw_text: String,
    pub snr: i32,
    pub freq_hz: f32,
    pub time_sec: f32,
    pub score: i32,
    pub i3: u8,
    pub n3: u8,
    /// 12-byte a91 (77-bit payload + 14-bit CRC), for deep-decode subtraction.
    pub a91: [u8; 12],
    /// True if this message only decoded via a priori ("MyCall known")
    /// assistance, not blindly -- mirrors WSJT-X's own `a2` tag. See
    /// `Decoder::set_my_call`.
    pub ap: bool,
}

pub struct Decoder {
    mon: ffi::monitor_t,
    candidates: Vec<ffi::candidate_t>,
    num_candidates: usize,
    ldpc_iters: i32,
    deep: bool,
    /// Shared (not exclusively thread-owned) so `decode_all` can fan its
    /// candidate loop across worker threads that all need to resolve each
    /// other's hashed/compound callsigns within one slot -- see
    /// `dsp/hashtable.rs`'s module doc comment.
    hashtable: Mutex<HashTable>,
    /// The 29-bit a priori encoding of our own callsign in a standard
    /// message's call_to field (payload bits 0..28), if computable --
    /// see `set_my_call`. `None` until set, or if the configured
    /// callsign can't be packed at all (never happens for a real call).
    ap_bits: Option<[u8; 29]>,
    /// Worker threads `decode_all` may use (always >= 1) -- see
    /// `set_worker_threads`/`decode_worker_count`.
    workers: usize,
}

// SAFETY: `ffi::monitor_t`'s raw-pointer fields block the auto-derived
// `Sync` impl, but every access to a shared `&Decoder` during `decode_all`'s
// scoped parallel section (via `decode_candidates`) is read-only on every
// field except `hashtable`, which is itself already `Sync` (a `Mutex`). No
// other field is ever touched except through an `&mut self` method
// (`feed_slot`, `find_sync`, `set_deep`, `set_my_call`,
// `set_worker_threads`), none of which can run while `decode_all`'s own
// `&mut self` borrow -- and therefore its scoped threads -- are alive.
unsafe impl Sync for Decoder {}

impl Decoder {
    /// Create a decoder for the given sample rate and protocol (one of
    /// `ffi::FTX_PROTOCOL_FT8`/`_FT4`/`_FT2` -- see `Mode::ffi_protocol`).
    /// `monitor_init` (common/monitor.c) derives the slot_time/symbol_period
    /// for the given protocol internally, so no other field here changes
    /// per mode.
    pub fn new(sample_rate: i32, protocol: std::os::raw::c_int) -> Self {
        // SAFETY: monitor_init fully initializes every field of `mon`.
        let mut mon: ffi::monitor_t = unsafe { std::mem::zeroed() };
        let cfg = ffi::monitor_config_t {
            f_min: 100.0,
            f_max: 3500.0,
            sample_rate,
            // 4 halves worst-case symbol-timing error; the largest measured
            // recall gain. freq_osr must stay 2 (4 regresses — see decode_params.h).
            time_osr: 4,
            freq_osr: 2,
            protocol,
        };
        unsafe { ffi::monitor_init(&mut mon, &cfg) };

        Decoder {
            mon,
            candidates: vec![ffi::candidate_t::default(); MAX_CANDIDATES],
            num_candidates: 0,
            ldpc_iters: LDPC_ITERS_NORMAL,
            deep: false,
            hashtable: Mutex::new(HashTable::new()),
            ap_bits: None,
            workers: 1,
        }
    }

    /// Set how many worker threads `decode_all` may fan its candidate loop
    /// out across (clamped to at least 1 -- a caller can never accidentally
    /// disable decoding by configuring 0). See `decode_worker_count` for
    /// deriving this from a configured core-reservation.
    pub fn set_worker_threads(&mut self, n: usize) {
        self.workers = n.max(1);
    }

    /// Set deep-decode mode (more LDPC iterations + a lower sync-score
    /// threshold in `find_sync`). Mirrors `setDecodeMode`.
    pub fn set_deep(&mut self, deep: bool) {
        self.deep = deep;
        self.ldpc_iters = if deep {
            LDPC_ITERS_DEEP
        } else {
            LDPC_ITERS_NORMAL
        };
    }

    /// Cache the a priori ("MyCall known") hypothesis for `call`: the 29
    /// bits a standard message's call_to field would hold if addressed to
    /// us. Computed once here (not per-candidate) since it only changes
    /// when the operator's own callsign does. A candidate whose blind
    /// decode fails is retried assuming this hypothesis -- see
    /// `decode_all`. No-op (clears any previous hypothesis) if `call`
    /// can't be packed at all, which never happens for a real callsign.
    pub fn set_my_call(&mut self, call: &str) {
        let c_call = match std::ffi::CString::new(call) {
            Ok(s) => s,
            Err(_) => {
                self.ap_bits = None;
                return;
            }
        };
        let mut bits = [0u8; 29];
        let ok = unsafe { ffi::ftx_pack_call_ap_bits(c_call.as_ptr(), bits.as_mut_ptr()) };
        self.ap_bits = if ok { Some(bits) } else { None };
    }

    /// Feed a full slot of mono f32 samples (12 kHz) through the monitor.
    /// Mirrors `feed()` in ft8_decoder.cpp: reset then process block by block.
    pub fn feed_slot(&mut self, samples: &[f32]) {
        let block = self.mon.block_size as usize;
        if block == 0 {
            return;
        }
        unsafe { ffi::monitor_reset(&mut self.mon) };
        // Reset the callsign hash cache per slot: hashes resolve only against
        // calls seen "earlier in the same slot", and a never-cleared table
        // would grow unbounded across slots on this long-lived decoder (the
        // Android path allocates a fresh table per slot; iOS clears here too).
        self.hashtable.lock().clear();
        let n = samples.len();
        let mut pos = 0;
        while pos + block <= n {
            unsafe { ffi::monitor_process(&mut self.mon, samples[pos..].as_ptr()) };
            pos += block;
        }
    }

    /// Locate sync candidates. Mirrors `DecoderFt8FindSync`.
    pub fn find_sync(&mut self) -> usize {
        let min_score = if self.deep { MIN_SCORE_DEEP } else { MIN_SCORE_FAST };
        let n = unsafe {
            ffi::ft8_find_sync(
                &self.mon.wf,
                MAX_CANDIDATES as i32,
                self.candidates.as_mut_ptr(),
                min_score,
            )
        };
        self.num_candidates = n.clamp(0, MAX_CANDIDATES as i32) as usize;
        self.num_candidates
    }

    /// Decode every candidate found by the last `find_sync`, returning the
    /// successfully decoded messages. Mirrors the per-candidate
    /// `DecoderFt8Analysis` loop -- optionally fanned out across
    /// `self.workers` threads (see `set_worker_threads`), each processing a
    /// contiguous slice of candidate indices. All workers share one hash
    /// table for the slot (`decode_candidates`), so a candidate on one
    /// thread can still resolve a hashed/compound callsign another
    /// candidate saved on a different thread, the same cross-referencing
    /// a single sequential pass already relied on.
    pub fn decode_all(&mut self) -> Vec<DecodedMessage> {
        let n = self.num_candidates;
        // Reborrow as shared: nothing below needs &mut, and a shared
        // reference is what the scoped worker threads need to capture.
        let self_ref: &Decoder = self;

        let chunks: Vec<Vec<DecodedMessage>> = if self.workers <= 1 || n < MIN_CANDIDATES_TO_PARALLELIZE
        {
            vec![self_ref.decode_candidates(0..n)]
        } else {
            let workers = self.workers.min(n);
            let chunk_len = (n + workers - 1) / workers;
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..workers)
                    .map(|w| {
                        let start = w * chunk_len;
                        let end = (start + chunk_len).min(n);
                        scope.spawn(move || self_ref.decode_candidates(start..end))
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            })
        };

        // The sync search yields several candidates for the same signal;
        // collapse those that decode to identical text so each message
        // appears once. Chunks are contiguous, increasing-index ranges
        // merged in that same order, so "first occurrence wins" here ties
        // identically to the old single sequential scan over 0..n.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out = Vec::new();
        for chunk in chunks {
            for msg in chunk {
                if msg.raw_text.is_empty() || seen.insert(msg.raw_text.clone()) {
                    out.push(msg);
                }
            }
        }
        out
    }

    /// Decode candidates `indices` (a sub-range of the last `find_sync`'s
    /// results), returning the successfully decoded messages in index
    /// order. Pulled out of `decode_all` so the exact same per-candidate
    /// logic runs whether called inline (no worker threads) or on a worker
    /// thread handling one slice of the candidate list.
    fn decode_candidates(&self, indices: std::ops::Range<usize>) -> Vec<DecodedMessage> {
        let mut out = Vec::new();
        // Activate this slot's shared hash table for the callbacks invoked
        // by ftx_message_decode*, for the lifetime of this call/thread.
        let table_ptr: *const Mutex<HashTable> = &self.hashtable;
        let _guard = unsafe { HashGuard::new(table_ptr) };
        let mut iface = super::hashtable::interface();

        for idx in indices {
            let cand = self.candidates[idx];
            let mut message = ffi::ftx_message_t {
                payload: [0; 10],
                hash: 0,
            };
            let mut status = ffi::decode_status_t {
                ldpc_errors: 0,
                crc_extracted: 0,
                crc_calculated: 0,
            };
            let mut ok = unsafe {
                ffi::ft8_decode(
                    &self.mon.wf,
                    &cand,
                    self.ldpc_iters,
                    &mut message,
                    &mut status,
                )
            };
            // A priori retry: blind decode failed, but this candidate
            // might be a message addressed to us -- same candidate (same
            // waterfall position), forcing the call_to field to our own
            // callsign's known encoding. The existing CRC check inside
            // ft8_decode_ap is the safety net against a wrong hypothesis
            // manufacturing a false decode.
            let mut via_ap = false;
            if !ok {
                if let Some(ap_bits) = self.ap_bits {
                    ok = unsafe {
                        ffi::ft8_decode_ap(
                            &self.mon.wf,
                            &cand,
                            self.ldpc_iters,
                            0,
                            0,
                            ap_bits.as_ptr(),
                            &mut message,
                            &mut status,
                        )
                    };
                    via_ap = ok;
                }
            }
            if !ok {
                continue;
            }

            if let Some(mut msg) = self.build_message(&cand, &message, &mut iface) {
                msg.ap = via_ap;
                out.push(msg);
            }
        }
        out
    }

    /// Coherent time-domain subtraction of an already-decoded message from
    /// `samples` (modified in place) -- the "Deep" tier's subtract-and-
    /// redecode loop (see `engine.rs`'s decoder-thread loop). `&self.mon`
    /// is read-only here (its STFT geometry only, per
    /// `ft8_subtract_signal_time`'s own contract) -- the caller must
    /// `feed_slot` the modified buffer back through before the next
    /// `find_sync`/`decode_all` pass to rebuild the STFT from the
    /// residual. Returns `false` on degenerate input (samples then
    /// unchanged) -- matches the native function's own contract exactly.
    pub fn subtract(&self, samples: &mut [f32], msg: &DecodedMessage, sample_rate: i32) -> bool {
        unsafe {
            ffi::ft8_subtract_signal_time(
                &self.mon,
                samples.as_mut_ptr(),
                samples.len() as i32,
                sample_rate,
                msg.a91.as_ptr(),
                msg.freq_hz,
                msg.time_sec,
            )
        }
    }

    /// Downsample the monitor's waterfall magnitudes (populated by the last
    /// `feed_slot`) into a `rows x cols` heatmap of 0..=255 values for display.
    /// Returns (bins, rows, cols, hz_per_col).
    pub fn waterfall_heatmap(&self, max_rows: usize, max_cols: usize) -> (Vec<u8>, usize, usize, f32) {
        let wf = &self.mon.wf;
        if wf.mag.is_null() || wf.num_blocks <= 0 || wf.num_bins <= 0 {
            return (Vec::new(), 0, 0, 0.0);
        }
        let num_blocks = wf.num_blocks as usize;
        let num_bins = wf.num_bins as usize;
        let stride = wf.block_stride as usize;
        let rows = num_blocks.min(max_rows).max(1);
        let cols = num_bins.min(max_cols).max(1);
        let mut bins = vec![0u8; rows * cols];
        for r in 0..rows {
            let block = r * num_blocks / rows;
            let base = block * stride; // time_sub=0, freq_sub=0 plane
            for c in 0..cols {
                let bin0 = c * num_bins / cols;
                let bin1 = (((c + 1) * num_bins / cols).max(bin0 + 1)).min(num_bins);
                let mut m = 0u8;
                for bin in bin0..bin1 {
                    let v = unsafe { *wf.mag.add(base + bin) };
                    if v > m {
                        m = v;
                    }
                }
                bins[r * cols + c] = m;
            }
        }
        // Bin spacing in Hz = (1/symbol_period)/freq_osr; columns aggregate bins.
        let bin_hz = (1.0 / self.mon.symbol_period) / wf.freq_osr as f32;
        let hz_per_col = bin_hz * (num_bins as f32 / cols as f32);
        (bins, rows, cols, hz_per_col)
    }

    fn build_message(
        &self,
        cand: &ffi::candidate_t,
        message: &ffi::ftx_message_t,
        iface: &mut ffi::ftx_callsign_hash_interface_t,
    ) -> Option<DecodedMessage> {
        let freq_hz = (self.mon.min_bin as f32
            + cand.freq_offset as f32
            + cand.freq_sub as f32 / self.mon.wf.freq_osr as f32)
            / self.mon.symbol_period;
        let time_sec = (cand.time_offset as f32
            + cand.time_sub as f32 / self.mon.wf.time_osr as f32)
            * self.mon.symbol_period;
        let snr = unsafe { ffi::ft8_snr(&self.mon.wf, cand) };

        let mut a91 = [0u8; 12];
        unsafe { ffi::ftx_add_crc(message.payload.as_ptr(), a91.as_mut_ptr()) };

        let i3 = unsafe { ffi::ftx_message_get_i3(message) };
        let n3 = unsafe { ffi::ftx_message_get_n3(message) };
        let msg_type = unsafe { ffi::ftx_message_get_type(message) };

        // Canonical full text (always available) — mirrors getMessageText output.
        let raw_text = {
            let mut buf = [0 as c_char; 64];
            let rc = unsafe { ffi::ftx_message_decode(message, iface, buf.as_mut_ptr()) };
            if rc == ffi::FTX_MESSAGE_RC_OK {
                cstr_to_string(&buf)
            } else {
                String::new()
            }
        };

        let mut m = DecodedMessage {
            snr,
            freq_hz,
            time_sec,
            score: cand.score as i32,
            i3,
            n3,
            a91,
            raw_text,
            ..Default::default()
        };

        match msg_type {
            ffi::FTX_MESSAGE_TYPE_STANDARD | ffi::FTX_MESSAGE_TYPE_NONSTD_CALL => {
                let mut call_to = [0 as c_char; 24];
                let mut call_de = [0 as c_char; 24];
                let mut extra = [0 as c_char; 24];
                let rc = unsafe {
                    if msg_type == ffi::FTX_MESSAGE_TYPE_STANDARD {
                        ffi::ftx_message_decode_std(
                            message,
                            iface,
                            call_to.as_mut_ptr(),
                            call_de.as_mut_ptr(),
                            extra.as_mut_ptr(),
                        )
                    } else {
                        ffi::ftx_message_decode_nonstd(
                            message,
                            iface,
                            call_to.as_mut_ptr(),
                            call_de.as_mut_ptr(),
                            extra.as_mut_ptr(),
                        )
                    }
                };
                if rc != ffi::FTX_MESSAGE_RC_OK {
                    return None;
                }
                m.call_to = cstr_to_string(&call_to);
                m.call_from = cstr_to_string(&call_de);
                m.extra = cstr_to_string(&extra);
                if looks_like_grid(&m.extra) {
                    m.grid = m.extra.clone();
                }
            }
            _ => {
                // Free text + contest sub-types: full text already in raw_text.
                m.extra = m.raw_text.clone();
            }
        }

        Some(m)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { ffi::monitor_free(&mut self.mon) };
    }
}

fn cstr_to_string(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn looks_like_grid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 4
        && (b'A'..=b'R').contains(&b[0])
        && (b'A'..=b'R').contains(&b[1])
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::encode::generate_signal;
    use crate::dsp::SAMPLE_RATE;
    use crate::mode::Mode;

    const NSAMPLES: usize = (SAMPLE_RATE as usize) * 15;

    /// Deterministic uniform noise in [-amp, amp] -- same LCG shape as
    /// `ft8af_glue/test_subtract.c`'s own `add_noise` (exact bit-for-bit
    /// reproduction isn't the point; a repeatable, non-degenerate noise
    /// floor is).
    fn add_noise(buf: &mut [f32], amp: f32, seed: u32) {
        let mut s = seed;
        for x in buf.iter_mut() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *x += amp * (2.0 * ((s >> 8) & 0xFFFF) as f32 / 65535.0 - 1.0);
        }
    }

    /// Encode `text` and mix its GFSK waveform into `buf` at (f0, dt, amp).
    fn add_message(buf: &mut [f32], text: &str, f0: f32, dt: f32, amp: f32) {
        let wave = generate_signal(text, f0, SAMPLE_RATE, Mode::Ft8).expect("text packs as a structured message");
        let off = (dt * SAMPLE_RATE as f32).round() as isize;
        for (i, &s) in wave.iter().enumerate() {
            let j = off + i as isize;
            if j >= 0 && (j as usize) < buf.len() {
                buf[j as usize] += amp * s;
            }
        }
    }

    fn decode_slot(decoder: &mut Decoder, samples: &[f32]) -> Vec<DecodedMessage> {
        decoder.feed_slot(samples);
        let n = decoder.find_sync();
        if n > 0 { decoder.decode_all() } else { Vec::new() }
    }

    #[test]
    fn set_my_call_matches_a_normally_encoded_messages_own_call_to_field() {
        // Cross-check ftx_pack_call_ap_bits against the existing, already-
        // proven encode path: a real message addressed to K1ABC has K1ABC
        // packed into payload bits 0..28 by ftx_message_encode_std -- the
        // new AP helper must compute the exact same 29 bits independently.
        let payload = crate::dsp::encode::pack77("K1ABC W9XYZ FN42").expect("packs as a structured message");
        let mut expected = [0u8; 29];
        for (i, bit) in expected.iter_mut().enumerate() {
            *bit = (payload[i / 8] >> (7 - i % 8)) & 1;
        }

        let mut decoder = Decoder::new(SAMPLE_RATE, Mode::Ft8.ffi_protocol());
        decoder.set_my_call("K1ABC");
        assert_eq!(decoder.ap_bits, Some(expected));
    }

    /// Build a GENUINE, valid 174-bit codeword for `text` (via the
    /// existing encode path + encode174, bypassing any waterfall/audio
    /// simulation entirely) and turn it into a log-likelihood array,
    /// with `corrupt_bits` (0-indexed payload-bit positions) each
    /// flipped to the WRONG confidence sign -- simulating a channel that
    /// confidently decided those specific bits incorrectly. Deterministic
    /// and precisely controllable, unlike fighting a noise amplitude
    /// against an opaque analog pipeline (see git history for why this
    /// replaced an amplitude-sweep attempt that empirically showed this
    /// decoder's BP is close to all-or-nothing against pure additive
    /// noise -- real benefit needs errors concentrated unevenly, which a
    /// hand-built codeword can guarantee deterministically).
    fn corrupted_codeword_llrs(text: &str, corrupt_bits: &[usize], strength: f32) -> ([u8; 10], [f32; 174]) {
        let payload = crate::dsp::encode::pack77(text).expect("packs as a structured message");
        let mut a91 = [0u8; 12];
        unsafe { ffi::ftx_add_crc(payload.as_ptr(), a91.as_mut_ptr()) };
        let mut codeword = [0u8; 22];
        unsafe { ffi::encode174(a91.as_ptr(), codeword.as_mut_ptr()) };

        let mut log174 = [0f32; 174];
        for (i, slot) in log174.iter_mut().enumerate() {
            let bit = (codeword[i / 8] >> (7 - i % 8)) & 1;
            let confident = if bit == 0 { -3.0 } else { 3.0 };
            // A corrupted bit leans the WRONG way at `strength` x a correct
            // bit's confidence -- lower strength (e.g. 0.2) is a weak/
            // ambiguous channel decision BP/OSD can realistically recover
            // from; strength 1.0 is a confident (and much harder to
            // recover from) wrong decision.
            *slot = if corrupt_bits.contains(&i) { -confident * strength } else { confident };
        }
        (payload, log174)
    }

    /// Payload bits 0..28 (the call_to field) of a standard message as a
    /// 29-entry 0/1 array -- the same convention `ftx_pack_call_ap_bits`
    /// uses, derived here independently from a known-good codeword for
    /// test construction.
    fn call_to_bits(codeword_text: &str) -> [u8; 29] {
        let payload = crate::dsp::encode::pack77(codeword_text).expect("packs as a structured message");
        let mut out = [0u8; 29];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (payload[i / 8] >> (7 - i % 8)) & 1;
        }
        out
    }

    /// Decode an ALREADY-normalized log174 (callers normalize first via
    /// `ffi::ftx_normalize_logl`, then optionally override specific bits
    /// -- normalizing after an override would skew the variance estimate
    /// using the override's own extreme values, exactly the pitfall
    /// `ft8_decode_ap`'s own doc comment warns about).
    fn decode_normalized_llrs(mut log174: [f32; 174]) -> (bool, ffi::ftx_message_t, i32) {
        let mut message = ffi::ftx_message_t { payload: [0; 10], hash: 0 };
        let mut status = ffi::decode_status_t { ldpc_errors: 0, crc_extracted: 0, crc_calculated: 0 };
        let ok = unsafe {
            ffi::ftx_decode_normalized_llrs(
                Mode::Ft8.ffi_protocol(),
                log174.as_mut_ptr(),
                30,
                0,
                0,
                &mut message,
                &mut status,
            )
        };
        (ok, message, status.ldpc_errors)
    }

    fn override_call_to(log174: &mut [f32; 174], bits: &[u8; 29]) {
        for (i, &bit) in bits.iter().enumerate() {
            log174[i] = if bit == 0 { -20.0 } else { 20.0 };
        }
    }

    // A handful of corrupted bits, used only by the negative-control test
    // below -- that test doesn't depend on defeating blind decode (the
    // CRC check it's exercising is a safety net regardless), just on
    // having *some* corruption present.
    const CORRUPT_BITS: &[usize] = &[2, 5, 8, 11, 14, 17, 20, 23, 26, 60];

    #[test]
    fn sanity_a_completely_clean_codeword_decodes() {
        // Confirms corrupted_codeword_llrs's own construction (encode174
        // + the empirically-confirmed log174 sign convention, see git
        // history: ldpc.c's "log(P(0)/P(1))" doc comment is stale/wrong,
        // contradicted by ft8_extract_symbol's "log(p(1)/p(0))") actually
        // round-trips -- a real regression guard for every other test in
        // this module that builds on it.
        let (payload, mut log174) = corrupted_codeword_llrs("K1ABC W9XYZ FN42", &[], 1.0);
        unsafe { ffi::ftx_normalize_logl(log174.as_mut_ptr()) };
        let (ok, message, _) = decode_normalized_llrs(log174);
        assert!(ok, "a zero-corruption codeword must decode -- if not, the LLR construction itself is wrong");
        assert_eq!(message.payload, payload);
    }

    #[test]
    fn ap_recovers_a_codeword_with_known_bits_corrupted_that_blind_decode_misses() {
        // 26 of the 29 call_to bits corrupted (90% -- blind has
        // essentially no idea what's really there) plus 5 bits scattered
        // across the rest of the codeword. A healthy, correctly-signed
        // LDPC(174,91) with 30 BP iterations turns out to be far more
        // robust than a first guess suggests (far smaller corruption
        // counts didn't defeat it) -- this is the real threshold where
        // blind genuinely fails, found by sweeping corruption count (see
        // git history for the sweep).
        let mut corrupt: Vec<usize> = (0..26).collect(); // 26/29 call_to bits
        corrupt.extend_from_slice(&[29, 58, 87, 116, 145]); // 5 bits elsewhere

        let (payload, mut log174) = corrupted_codeword_llrs("K1ABC W9XYZ FN42", &corrupt, 1.0);
        unsafe { ffi::ftx_normalize_logl(log174.as_mut_ptr()) };
        let (blind_ok, _, _) = decode_normalized_llrs(log174);
        assert!(
            !blind_ok,
            "this corruption pattern must defeat blind decode (otherwise this test proves nothing)"
        );

        let mut ap_llrs = log174;
        override_call_to(&mut ap_llrs, &call_to_bits("K1ABC W9XYZ FN42"));
        let (ap_ok, message, _) = decode_normalized_llrs(ap_llrs);
        assert!(ap_ok, "AP (call_to restored to its known value) must recover this same codeword via plain BP");
        assert_eq!(message.payload, payload, "the recovered payload must be the real message, not a CRC coincidence");
    }

    #[test]
    fn ap_with_the_wrong_hypothesis_does_not_produce_a_false_decode() {
        // Same corrupted codeword, but the AP override forces call_to to
        // a DIFFERENT callsign than the one actually in the message --
        // the wrong hypothesis must not satisfy CRC. This is what makes
        // AP safe: forcing bits alone can't manufacture a decode, only
        // the CRC check over the remaining bits can confirm one.
        let (_, mut log174) = corrupted_codeword_llrs("K1ABC W9XYZ FN42", CORRUPT_BITS, 1.0);
        unsafe { ffi::ftx_normalize_logl(log174.as_mut_ptr()) };
        let mut wrong_llrs = log174;
        override_call_to(&mut wrong_llrs, &call_to_bits("W9XYZ K1ABC FN42"));
        let (wrong_ok, _, _) = decode_normalized_llrs(wrong_llrs);
        assert!(!wrong_ok, "a wrong a priori hypothesis must not produce a false decode");
    }

    #[test]
    fn blind_decodes_are_never_tagged_ap() {
        // End-to-end (through Decoder::decode_all, not the raw-LLR path
        // above) sanity check that the `ap` tag itself is wired correctly
        // on a real, strong, blind-decodable signal.
        let mut signal = vec![0.0f32; NSAMPLES];
        add_noise(&mut signal, 0.02, 51);
        add_message(&mut signal, "CQ K1ABC FN42", 1500.0, 0.5, 1.0);

        let mut decoder = Decoder::new(SAMPLE_RATE, Mode::Ft8.ffi_protocol());
        decoder.set_deep(true);
        decoder.set_my_call("K1ABC");
        let pass = decode_slot(&mut decoder, &signal);
        let msg = pass.iter().find(|m| m.raw_text.contains("K1ABC"));
        assert!(msg.is_some());
        assert!(!msg.unwrap().ap, "a signal strong enough to decode blind must not be tagged ap");
    }

    #[test]
    fn subtract_recovers_a_weak_signal_masked_by_a_strong_one_at_nearly_the_same_frequency() {
        // Direct port of `ft8af_glue/test_subtract.c`'s
        // "test_td_same_freq_recovery" (property 6, the real reason the
        // time-domain subtraction path exists over a waterfall-domain
        // one): B sits 2 Hz above A -- deep inside every tone bin A
        // occupies -- 10 dB weaker, and shifted 0.7s in time. No
        // waterfall-domain trick can separate them; only coherent
        // sample-domain subtraction can. This is the actual mechanism the
        // "Deep" decode tier's subtract-and-redecode loop (`engine.rs`'s
        // `spawn_decoder_thread`) exists to exploit.
        let mut signal = vec![0.0f32; NSAMPLES];
        add_noise(&mut signal, 0.02, 47);
        add_message(&mut signal, "CQ K1ABC FN42", 1500.0, 0.2, 1.0);
        add_message(&mut signal, "CQ W9XYZ EN37", 1502.0, 0.9, 0.3);

        let mut decoder = Decoder::new(SAMPLE_RATE, Mode::Ft8.ffi_protocol());
        decoder.set_deep(true);

        let pass0 = decode_slot(&mut decoder, &signal);
        let a = pass0.iter().find(|m| m.raw_text.contains("K1ABC"));
        assert!(a.is_some(), "strong signal must decode in pass 1");
        assert!(
            !pass0.iter().any(|m| m.raw_text.contains("W9XYZ")),
            "weak same-frequency signal must be masked in pass 1 (otherwise this test proves nothing)"
        );
        let a = a.unwrap().clone();

        assert!(
            decoder.subtract(&mut signal, &a, SAMPLE_RATE),
            "subtraction must report success"
        );

        let pass1 = decode_slot(&mut decoder, &signal);
        assert!(
            pass1.iter().any(|m| m.raw_text.contains("W9XYZ")),
            "masked same-frequency signal must be recovered after subtraction"
        );
        assert!(
            !pass1.iter().any(|m| m.raw_text.contains("K1ABC")),
            "the strong signal itself must be gone after being subtracted"
        );
    }

    #[test]
    fn subtract_leaves_a_neighbor_100hz_away_undamaged() {
        // Port of test_subtract.c's "test_td_neighbor_survives": subtracting
        // one signal must not collaterally damage an unrelated one well
        // outside its own tone track.
        let mut signal = vec![0.0f32; NSAMPLES];
        add_noise(&mut signal, 0.02, 48);
        add_message(&mut signal, "CQ K1ABC FN42", 1500.0, 0.5, 1.0);
        add_message(&mut signal, "CQ W9XYZ EN37", 1600.0, 0.5, 0.5);

        let mut decoder = Decoder::new(SAMPLE_RATE, Mode::Ft8.ffi_protocol());
        decoder.set_deep(true);

        let pass0 = decode_slot(&mut decoder, &signal);
        let a = pass0.iter().find(|m| m.raw_text.contains("K1ABC"));
        assert!(a.is_some() && pass0.iter().any(|m| m.raw_text.contains("W9XYZ")), "both decode initially");
        let a = a.unwrap().clone();

        decoder.subtract(&mut signal, &a, SAMPLE_RATE);
        let pass1 = decode_slot(&mut decoder, &signal);
        assert!(pass1.iter().any(|m| m.raw_text.contains("W9XYZ")), "neighbor still decodes after subtracting A");
        assert!(!pass1.iter().any(|m| m.raw_text.contains("K1ABC")), "A itself is gone");
    }

    #[test]
    fn parallel_decode_matches_sequential_decode_exactly() {
        // The core correctness guarantee of candidate-level decode
        // parallelism: splitting the same candidate list across worker
        // threads must produce EXACTLY the same result (same messages,
        // same order, same dedup) as running single-threaded -- the
        // worker count only changes how the work is scheduled, never what
        // it computes.
        let mut signal = vec![0.0f32; NSAMPLES];
        add_noise(&mut signal, 0.05, 7);
        let calls = [
            ("K1AAA", "FN42"),
            ("K1BBB", "FN43"),
            ("K1CCC", "FN44"),
            ("K1DDD", "FN45"),
            ("K1EEE", "FN46"),
            ("K1FFF", "FN47"),
            ("K1GGG", "FN48"),
            ("K1HHH", "FN49"),
            ("K1III", "FN50"),
            ("K1JJJ", "FN51"),
        ];
        for (i, (call, grid)) in calls.iter().enumerate() {
            let f0 = 300.0 + 300.0 * i as f32;
            add_message(&mut signal, &format!("CQ {call} {grid}"), f0, 0.5, 1.0);
        }

        let mut sequential = Decoder::new(SAMPLE_RATE, Mode::Ft8.ffi_protocol());
        sequential.set_deep(true);
        sequential.set_worker_threads(1);
        sequential.feed_slot(&signal);
        let n = sequential.find_sync();
        assert!(
            n >= MIN_CANDIDATES_TO_PARALLELIZE,
            "fixture must yield enough candidates to actually exercise the \
             parallel chunking path (got {n}, need >= {MIN_CANDIDATES_TO_PARALLELIZE})"
        );
        let seq_out = sequential.decode_all();

        let mut parallel = Decoder::new(SAMPLE_RATE, Mode::Ft8.ffi_protocol());
        parallel.set_deep(true);
        parallel.set_worker_threads(4);
        parallel.feed_slot(&signal);
        parallel.find_sync();
        let par_out = parallel.decode_all();

        // Sanity: the fixture must actually produce every message, or this
        // test would pass vacuously by both sides finding nothing.
        for (call, _) in &calls {
            assert!(
                seq_out.iter().any(|m| m.raw_text.contains(call)),
                "fixture signal for {call} must actually decode (test is vacuous otherwise)"
            );
        }

        let seq_texts: Vec<&str> = seq_out.iter().map(|m| m.raw_text.as_str()).collect();
        let par_texts: Vec<&str> = par_out.iter().map(|m| m.raw_text.as_str()).collect();
        assert_eq!(
            seq_texts, par_texts,
            "worker count must never change decode_all's output (messages or their order)"
        );
    }

    #[test]
    fn decode_worker_count_never_returns_zero() {
        // A reservation that eats every detected core (or exceeds it) must
        // still leave exactly one worker -- this setting can never
        // accidentally disable decoding.
        assert!(decode_worker_count(0) >= 1);
        assert_eq!(decode_worker_count(u32::MAX), 1);
    }

    #[test]
    fn decode_worker_count_subtracts_the_reservation() {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        if cores > 1 {
            assert_eq!(decode_worker_count(1), (cores - 1).max(1));
        } else {
            assert_eq!(decode_worker_count(1), 1);
        }
        assert_eq!(decode_worker_count(0), cores);
    }
}
