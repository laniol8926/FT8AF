package com.k1af.ft8af;

import static com.google.common.truth.Truth.assertThat;

import androidx.test.core.app.ApplicationProvider;

import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;

/**
 * {@link GeneralVariables#workedStateFor}: a logged ADIF STATE is used in preference to the
 * grid-derived state, but only when it is a US state on a US station, since STATE also holds
 * other countries' subdivisions (e.g. Western Australia "WA").
 *
 * <p>Robolectric so the bundled {@code us_grid_states.json} asset can be read through the
 * main context, as on device.
 */
@RunWith(RobolectricTestRunner.class)
public class GeneralVariablesWorkedStateForTest {

    @Before
    public void setUp() {
        GeneralVariables.getInstance().setMainContext(ApplicationProvider.getApplicationContext());
    }

    @Test
    public void noLoggedState_fallsBackToTheGrid() {
        String fromGrid = GeneralVariables.stateForGrid("EM53");
        assertThat(fromGrid).isNotNull();
        assertThat(GeneralVariables.workedStateFor(null, "EM53", "W5ORC")).isEqualTo(fromGrid);
    }

    @Test
    public void loggedUsState_winsOverTheGrid() {
        // FM29 is assigned to one state in the grid table; a logged DE overrides it, which the
        // grid table alone can never produce.
        assertThat(GeneralVariables.workedStateFor("de", "FM29", "K3ABC")).isEqualTo("DE");
    }

    @Test
    public void loggedStateOnANonUsStation_isIgnored() {
        // Western Australia logged as STATE=WA with an Australian grid: not Washington.
        assertThat(GeneralVariables.workedStateFor("WA", "OF78", "VK6ABC")).isNull();
        // No grid either: the non-US callsign keeps it from counting.
        assertThat(GeneralVariables.workedStateFor("WA", "", "VK6ABC")).isNull();
    }

    @Test
    public void gridlessUsCallsign_usesTheLoggedState() {
        assertThat(GeneralVariables.workedStateFor("RI", "", "W1XYZ")).isEqualTo("RI");
        assertThat(GeneralVariables.workedStateFor("HI", null, "KH6/W1XYZ")).isEqualTo("HI");
        assertThat(GeneralVariables.workedStateFor("TX", "", "AA5B/P")).isEqualTo("TX");
    }

    @Test
    public void nonStateCodes_areIgnored() {
        // A Canadian province with a gridless call, and junk.
        assertThat(GeneralVariables.workedStateFor("ON", "", "VE3ABC")).isNull();
        assertThat(GeneralVariables.workedStateFor("XX", "", "W1XYZ")).isNull();
    }
}
