package com.k1af.ft8af;

import static com.google.common.truth.Truth.assertThat;

import org.junit.Test;

/** {@link GeneralVariables#formatWatts}: the operator power as shown and saved, not rounded. */
public class GeneralVariablesFormatWattsTest {

    @Test
    public void keepsTenthsOfAWatt() {
        assertThat(GeneralVariables.formatWatts(4.5f)).isEqualTo("4.5");
        assertThat(GeneralVariables.formatWatts(12.4f)).isEqualTo("12.4");
        assertThat(GeneralVariables.formatWatts(0.8f)).isEqualTo("0.8");
    }

    @Test
    public void wholeWattsHaveNoDecimal() {
        assertThat(GeneralVariables.formatWatts(5f)).isEqualTo("5");
        assertThat(GeneralVariables.formatWatts(100f)).isEqualTo("100");
    }

    @Test
    public void floatNoiseDoesNotShow() {
        assertThat(GeneralVariables.formatWatts(45 / 10f)).isEqualTo("4.5");
        assertThat(GeneralVariables.formatWatts(4.4999998f)).isEqualTo("4.5");
    }
}
