//! M1 acceptance gate: a Rust port of ft8af_glue/test_roundtrip.c.
//!
//! Proves the C ft8_lib core links and is self-consistent through the Rust FFI:
//! pack a known message -> encode to tones -> synthesize the GFSK waveform ->
//! feed it back through the monitor + sync search + LDPC decoder and confirm the
//! original text is recovered. This is the gate for the whole project.

use ft8af::dsp::{encode, ffi, hashtable, Decoder, SAMPLE_RATE, SLOT_SAMPLES};

#[test]
fn encode_then_decode_recovers_message() {
    let expected = "CQ K1ABC FN42";
    let freq_hz = 1000.0;

    // 1. pack -> 2. encode -> 3. synth GFSK into a full 15 s slot buffer.
    let payload = encode::pack77(expected).expect("pack77 should accept a standard CQ");
    let tones = encode::encode_tones(&payload);
    let mut signal = vec![0.0f32; SLOT_SAMPLES];
    encode::synth_gfsk(&tones, freq_hz, SAMPLE_RATE, &mut signal, 0);

    // 4-5. monitor -> find_sync -> decode.
    let mut decoder = Decoder::new(SAMPLE_RATE, ffi::FTX_PROTOCOL_FT8);
    decoder.feed_slot(&signal);
    let n = decoder.find_sync();
    assert!(n > 0, "expected at least one sync candidate, got {n}");

    let messages = decoder.decode_all();
    assert!(
        !messages.is_empty(),
        "decoder found {n} candidates but decoded nothing"
    );

    let recovered = messages.iter().find(|m| m.raw_text == expected);
    assert!(
        recovered.is_some(),
        "original message not recovered; decoded texts: {:?}",
        messages.iter().map(|m| &m.raw_text).collect::<Vec<_>>()
    );

    let m = recovered.unwrap();
    assert_eq!(m.call_to, "CQ");
    assert_eq!(m.call_from, "K1ABC");
    assert_eq!(m.grid, "FN42");
    // The signal was synthesized at 1000 Hz; decoder should land close.
    assert!(
        (m.freq_hz - freq_hz).abs() < 10.0,
        "frequency off: {} Hz",
        m.freq_hz
    );
}

/// Regression guard for a correctness concern raised about the callsign/
/// grid cache (`Db::upsert_callsign_qth_live`, fed from `engine.rs::
/// publish_decodes`'s `m.call_from`/`m.grid`): a directional CQ like
/// "CQ POTA K1ABC FN42" must never let "POTA" leak into the recorded
/// callsign or grid.
///
/// Confirmed by reading `ft8_lib/ft8/message.c`'s `unpack28` (the function
/// that decodes the FIRST c28 field, i.e. `call_to`): n28 values in
/// 1003..=532443 are the "CQ ABCD" directional-token range (up to 4
/// alphanumeric symbols from `FT8_CHAR_TABLE_LETTERS_SPACE`, " ABCDEFGH...Z"),
/// decoded as e.g. "CQ POTA" into the `call_to` buffer *only* -- `call_de`
/// (-> `call_from`) and the grid come from the second c28 field and the
/// separate grid field respectively, decoded independently by
/// `ftx_message_decode_std` and never touched by the first field's value.
/// `pack77` itself can't encode "CQ_POTA"-style input (`message.c`'s
/// `pack28` has a literal `// TODO: ... not implemented` for it), so this
/// test builds the 77-bit payload directly: pack a real, ordinary message
/// to get a valid call_de/grid bit pattern, then overwrite just the call_to
/// field's bits with the directional-CQ encoding computed by hand from
/// `unpack28`'s own formula, and decode through the real C function.
#[test]
fn directional_cq_modifier_does_not_leak_into_call_from_or_grid() {
    // "CQ POTA": n28 = 1003 + sum(char_value(c) * 27^(3-i)) for c in "POTA",
    // table " ABCDEFGHIJKLMNOPQRSTUVWXYZ" (space=0, A=1, ..., Z=26).
    let value = |c: char| -> u32 {
        if c == ' ' { 0 } else { (c as u8 - b'A') as u32 + 1 }
    };
    let n28_cq_pota = 1003
        + value('P') * 27 * 27 * 27
        + value('O') * 27 * 27
        + value('T') * 27
        + value('A');
    assert!((1003..=532_443).contains(&n28_cq_pota), "must land in the CQ ABCD range");
    let n29a_new = n28_cq_pota << 1; // ip (the /P or /R flag) = 0

    // A real, fully-valid payload for an ordinary standard message -- only
    // its call_to (bytes 0..3 + byte 3's top 5 bits) gets overwritten below;
    // call_de ("K1ABC") and the grid ("FN42") bits are untouched.
    let real = encode::pack77("W9ZZZ K1ABC FN42").expect("pack77 should accept a standard message");
    let mut payload = real;
    payload[0] = ((n29a_new >> 21) & 0xFF) as u8;
    payload[1] = ((n29a_new >> 13) & 0xFF) as u8;
    payload[2] = ((n29a_new >> 5) & 0xFF) as u8;
    // Top 5 bits of byte 3 come from n29a's low 5 bits; the bottom 3 bits
    // are the start of n29b (call_de) and must survive from `real` untouched.
    payload[3] = (((n29a_new & 0x1F) << 3) as u8) | (real[3] & 0x07);

    let msg = ffi::ftx_message_t { payload, hash: 0 };
    let mut table = Box::new(hashtable::HashTable::new());
    let table_ptr: *mut hashtable::HashTable = &mut *table;
    // SAFETY: `table` outlives the guard and the decode_std call below.
    let _guard = unsafe { hashtable::HashGuard::new(table_ptr) };
    let mut iface = hashtable::interface();

    let mut call_to = [0 as std::os::raw::c_char; 24];
    let mut call_de = [0 as std::os::raw::c_char; 24];
    let mut extra = [0 as std::os::raw::c_char; 24];
    let rc = unsafe {
        ffi::ftx_message_decode_std(
            &msg,
            &mut iface,
            call_to.as_mut_ptr(),
            call_de.as_mut_ptr(),
            extra.as_mut_ptr(),
        )
    };
    assert_eq!(rc, ffi::FTX_MESSAGE_RC_OK, "decode_std should accept the hand-built payload");

    let to_string = |buf: &[std::os::raw::c_char]| -> String {
        let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    assert_eq!(to_string(&call_to), "CQ POTA", "the directional token stays part of call_to");
    assert_eq!(to_string(&call_de), "K1ABC", "the actual station must not be polluted by the modifier");
    assert_eq!(to_string(&extra), "FN42", "the grid must not be polluted by the modifier");
}

#[test]
fn generate_ft8_produces_expected_length() {
    let signal = encode::generate_ft8("CQ K1ABC FN42", 1000.0, SAMPLE_RATE).unwrap();
    // 79 symbols * 0.16 s * 12000 Hz = 151680 samples (+ rounding).
    assert_eq!(signal.len(), 151680);
}
