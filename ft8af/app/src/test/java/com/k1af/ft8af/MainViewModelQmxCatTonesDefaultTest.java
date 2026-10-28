package com.k1af.ft8af;

import static com.google.common.truth.Truth.assertThat;

import com.k1af.ft8af.rigs.HamlibRig;

import org.junit.Test;

/** {@link MainViewModel#shouldDefaultQmxCatTonesOn}: on for a QMX, never over a user choice. */
public class MainViewModelQmxCatTonesDefaultTest {

    @Test
    public void qmxWithNoPriorChoice_turnsCatTonesOn() {
        assertThat(MainViewModel.shouldDefaultQmxCatTonesOn(
                HamlibRig.QRPLABS_QMX_MODEL_ID, false)).isTrue();
    }

    @Test
    public void qmxWithAnExplicitChoice_isLeftAlone() {
        // A config row exists: the operator set it (on or off); don't override.
        assertThat(MainViewModel.shouldDefaultQmxCatTonesOn(
                HamlibRig.QRPLABS_QMX_MODEL_ID, true)).isFalse();
    }

    @Test
    public void otherRigs_areLeftAlone() {
        assertThat(MainViewModel.shouldDefaultQmxCatTonesOn(1, false)).isFalse();
        assertThat(MainViewModel.shouldDefaultQmxCatTonesOn(0, false)).isFalse();
    }
}
