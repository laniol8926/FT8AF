package com.k1af.ft8af.ft8transmit;

import static com.google.common.truth.Truth.assertThat;

import org.junit.Test;

/**
 * Coverage for the pure logic on the {@link TransmitCallsign} data holder:
 *   - {@link TransmitCallsign#haveTargetCallsign()} (null/"CQ" => no target)
 *   - {@link TransmitCallsign#getSnr()} (standard report form: sign and two digits)
 *
 * Plain JUnit: TransmitCallsign has no Android runtime dependency.
 */
public class TransmitCallsignTest {

    private static TransmitCallsign withCallsign(String call) {
        // 4-arg ctor: (i3, n3, callsign, sequential)
        return new TransmitCallsign(1, 0, call, 0);
    }

    private static TransmitCallsign withSnr(int snr) {
        // 6-arg ctor: (i3, n3, callsign, frequency, sequential, snr)
        return new TransmitCallsign(1, 0, "K1ABC", 1500f, 0, snr);
    }

    @Test
    public void haveTargetCallsign_nullCallsign_isFalse() {
        assertThat(withCallsign(null).haveTargetCallsign()).isFalse();
    }

    @Test
    public void haveTargetCallsign_cqCallsign_isFalse() {
        // Exactly "CQ" means we are calling generally, not a specific station.
        assertThat(withCallsign("CQ").haveTargetCallsign()).isFalse();
    }

    @Test
    public void haveTargetCallsign_realCallsign_isTrue() {
        assertThat(withCallsign("K1ABC").haveTargetCallsign()).isTrue();
    }

    @Test
    public void getSnr_positive_hasPlusSignAndTwoDigits() {
        assertThat(withSnr(7).getSnr()).isEqualTo("+07");
    }

    @Test
    public void getSnr_zero_isPlusZeroZero() {
        // Fork: the standard FT8 report form; a bare "0" made the non-standard "R0".
        assertThat(withSnr(0).getSnr()).isEqualTo("+00");
    }

    @Test
    public void getSnr_negative_keepsMinusSign() {
        assertThat(withSnr(-12).getSnr()).isEqualTo("-12");
        assertThat(withSnr(-4).getSnr()).isEqualTo("-04");
    }

    @Test
    public void sixArgConstructor_storesAllFields() {
        TransmitCallsign tc = new TransmitCallsign(2, 3, "VE3XY", 2500f, 1, 5);
        assertThat(tc.i3).isEqualTo(2);
        assertThat(tc.n3).isEqualTo(3);
        assertThat(tc.callsign).isEqualTo("VE3XY");
        assertThat(tc.frequency).isEqualTo(2500f);
        assertThat(tc.sequential).isEqualTo(1);
        assertThat(tc.snr).isEqualTo(5);
    }
}
