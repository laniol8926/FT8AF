package com.k1af.ft8af.rigs;

import static com.google.common.truth.Truth.assertThat;

import org.junit.Test;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/** {@link QmxMeterReplies}: SW replies are taken out of the stream, the rest goes to Hamlib. */
public class QmxMeterRepliesTest {

    private final StringBuilder pending = new StringBuilder();
    private final List<String> replies = new ArrayList<>();

    private String split(String in) {
        byte[] out = QmxMeterReplies.split(pending, in.getBytes(StandardCharsets.US_ASCII),
                replies::add);
        return new String(out, StandardCharsets.US_ASCII);
    }

    @Test
    public void swReply_isTakenOut() {
        assertThat(split("SW130;")).isEmpty();
        assertThat(replies).containsExactly("SW130;");
    }

    @Test
    public void otherReplies_passThroughUnchanged() {
        assertThat(split("ID020;IF00014074000;")).isEqualTo("ID020;IF00014074000;");
        assertThat(replies).isEmpty();
    }

    @Test
    public void mixedReplies_areSeparated() {
        assertThat(split("ID020;SW145;FA00014074000;")).isEqualTo("ID020;FA00014074000;");
        assertThat(replies).containsExactly("SW145;");
    }

    @Test
    public void swReplySplitAcrossReads_isReassembled() {
        assertThat(split("S")).isEmpty();
        assertThat(split("W1")).isEmpty();
        assertThat(split("30;")).isEmpty();
        assertThat(replies).containsExactly("SW130;");
    }

    @Test
    public void nonSwFragment_isPassedOnAtOnce() {
        assertThat(split("IF0001")).isEqualTo("IF0001");
        assertThat(pending.toString()).isEmpty();
    }

    @Test
    public void parseSwr_readsHundredths() {
        assertThat(QmxMeterReplies.parseSwr("SW130;")).isWithin(1e-4f).of(1.30f);
        assertThat(QmxMeterReplies.parseSwr("SW100;")).isWithin(1e-4f).of(1.00f);
        assertThat(QmxMeterReplies.parseSwr("SW1.45;")).isWithin(1e-4f).of(1.45f);
    }

    @Test
    public void parseSwr_rejectsJunk() {
        assertThat(QmxMeterReplies.parseSwr("SW;")).isNaN();
        assertThat(QmxMeterReplies.parseSwr("SWx;")).isNaN();
        assertThat(QmxMeterReplies.parseSwr("ID020;")).isNaN();
        assertThat(QmxMeterReplies.parseSwr(null)).isNaN();
    }

    @Test
    public void powerReply_isTakenOutToo() {
        assertThat(split("PC45;ID020;SW120;")).isEqualTo("ID020;");
        assertThat(replies).containsExactly("PC45;", "SW120;").inOrder();
    }

    @Test
    public void powerReplySplitAcrossReads_isReassembled() {
        assertThat(split("P")).isEmpty();
        assertThat(split("C4")).isEmpty();
        assertThat(split("5;")).isEmpty();
        assertThat(replies).containsExactly("PC45;");
    }

    @Test
    public void parsePower_readsTenthsOfAWatt() {
        assertThat(QmxMeterReplies.parsePowerWatts("PC45;")).isWithin(1e-4f).of(4.5f);
        assertThat(QmxMeterReplies.parsePowerWatts("PC0;")).isWithin(1e-4f).of(0f);
        assertThat(QmxMeterReplies.parsePowerWatts("SW120;")).isNaN();
    }
}
