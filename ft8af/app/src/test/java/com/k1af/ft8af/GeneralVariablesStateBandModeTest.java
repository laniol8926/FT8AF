package com.k1af.ft8af;

import static com.google.common.truth.Truth.assertThat;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;

/**
 * "NEW STATE" is per band and per mode: {@link GeneralVariables#isStateWorkedOn} only counts a
 * state as worked on the band/mode it was logged on. Before this, worked states were one
 * overall set, so a state worked on any band never showed as new again.
 *
 * <p>Robolectric because {@link GeneralVariables} carries Android LiveData statics.
 */
@RunWith(RobolectricTestRunner.class)
public class GeneralVariablesStateBandModeTest {

    @Before
    @After
    public void clear() {
        GeneralVariables.workedStateBandModes.clear();
    }

    @Test
    public void workedOnlyOnTheBandAndModeItWasLoggedOn() {
        GeneralVariables.addStateWorked("40m", "FT8", "MO");

        assertThat(GeneralVariables.isStateWorkedOn("40m", "FT8", "MO")).isTrue();
        // Same state, other band or other mode: still new.
        assertThat(GeneralVariables.isStateWorkedOn("20m", "FT8", "MO")).isFalse();
        assertThat(GeneralVariables.isStateWorkedOn("40m", "FT4", "MO")).isFalse();
        // Other state on the same band/mode: still new.
        assertThat(GeneralVariables.isStateWorkedOn("40m", "FT8", "AR")).isFalse();
    }

    @Test
    public void matchingIgnoresCaseAndPadding() {
        // The logbook has both "40m" and "40M".
        GeneralVariables.addStateWorked("40M", "ft8", " mo ");
        assertThat(GeneralVariables.isStateWorkedOn("40m", "FT8", "MO")).isTrue();
    }

    @Test
    public void incompleteRowsNeverCount() {
        GeneralVariables.addStateWorked(null, "FT8", "MO");
        GeneralVariables.addStateWorked("40m", "", "MO");
        GeneralVariables.addStateWorked("40m", "FT8", null);
        assertThat(GeneralVariables.workedStateBandModes).isEmpty();
        assertThat(GeneralVariables.stateBandModeKey("40m", "FT8", null)).isNull();
    }

    @Test
    public void currentModeNameMatchesTheLogsModeNames() {
        int saved = GeneralVariables.operatingMode;
        try {
            GeneralVariables.operatingMode = FT8Common.FT8_MODE;
            assertThat(GeneralVariables.currentModeName()).isEqualTo("FT8");
            GeneralVariables.operatingMode = FT8Common.FT4_MODE;
            assertThat(GeneralVariables.currentModeName()).isEqualTo("FT4");
        } finally {
            GeneralVariables.operatingMode = saved;
        }
    }
}
