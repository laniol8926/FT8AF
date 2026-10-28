package com.k1af.ft8af;

import static com.google.common.truth.Truth.assertThat;

import org.junit.Test;

/** {@link MainViewModel#shouldDefaultQmxCatTonesOn}: on for a QMX, never over a user choice. */
public class MainViewModelQmxCatTonesDefaultTest {

    @Test
    public void qmxWithNoPriorChoice_turnsCatTonesOn() {
        assertThat(MainViewModel.shouldDefaultQmxCatTonesOn(true, false)).isTrue();
    }

    @Test
    public void qmxWithAnExplicitChoice_isLeftAlone() {
        // A config row exists: the operator set it (on or off); don't override.
        assertThat(MainViewModel.shouldDefaultQmxCatTonesOn(true, true)).isFalse();
    }

    @Test
    public void otherRigs_areLeftAlone() {
        assertThat(MainViewModel.shouldDefaultQmxCatTonesOn(false, false)).isFalse();
    }
}
