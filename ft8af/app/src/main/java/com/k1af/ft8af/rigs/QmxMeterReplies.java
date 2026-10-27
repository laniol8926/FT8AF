package com.k1af.ft8af.rigs;

import java.nio.charset.StandardCharsets;

/**
 * Reads the QMX's SWR and power meters over CAT ({@code SW;} and {@code PC;},
 * firmware 1.04) alongside Hamlib, which owns the serial link and parses every
 * reply it is fed. While a reading is expected, the incoming bytes go through
 * {@link #split}: complete {@code SW...;} and {@code PC...;} replies are taken
 * out for the meters and everything else is handed on to Hamlib unchanged.
 *
 * <p>Pure, so the stream splitting and value parsing are unit-tested.
 */
final class QmxMeterReplies {
    /** The SWR meter query. */
    static final String SWR_QUERY = "SW;";
    /** The output power meter query. */
    static final String POWER_QUERY = "PC;";

    /** Receives each complete meter reply, e.g. {@code "SW130;"} or {@code "PC45;"}. */
    interface Sink {
        void onMeterReply(String reply);
    }

    private QmxMeterReplies() {
    }

    /**
     * Append {@code data} to {@code pending}, hand every complete meter reply to
     * {@code sink}, and return the bytes that belong to Hamlib. A trailing
     * fragment that could still be the start of a meter reply ("S", "SW1",
     * "PC") is held in {@code pending}; anything else is passed through at once.
     */
    static byte[] split(StringBuilder pending, byte[] data, Sink sink) {
        pending.append(new String(data, StandardCharsets.US_ASCII));
        StringBuilder forward = new StringBuilder();
        int end;
        while ((end = pending.indexOf(";")) >= 0) {
            String token = pending.substring(0, end + 1);
            pending.delete(0, end + 1);
            if (isMeterReply(token)) {
                sink.onMeterReply(token);
            } else {
                forward.append(token);
            }
        }
        if (pending.length() > 0 && !couldBeMeterPrefix(pending)) {
            forward.append(pending);
            pending.setLength(0);
        }
        return forward.toString().getBytes(StandardCharsets.US_ASCII);
    }

    static boolean isMeterReply(String token) {
        return token.startsWith("SW") || token.startsWith("PC");
    }

    private static boolean couldBeMeterPrefix(CharSequence s) {
        char first = s.charAt(0);
        if (first == 'S') return s.length() == 1 || s.charAt(1) == 'W';
        if (first == 'P') return s.length() == 1 || s.charAt(1) == 'C';
        return false;
    }

    /**
     * SWR ratio from a reply such as {@code "SW130;"}, or NaN when it isn't a
     * reading. The QMX reports the ratio times 100 (SW120; = 1.20:1, confirmed
     * on the radio); a value with a decimal point is taken as the ratio itself.
     */
    static float parseSwr(String reply) {
        return parseScaled(reply, "SW", 100f);
    }

    /**
     * Output power in watts from a reply such as {@code "PC45;"} (tenths of a
     * watt: 4.5 W), or NaN when it isn't a reading.
     */
    static float parsePowerWatts(String reply) {
        return parseScaled(reply, "PC", 10f);
    }

    private static float parseScaled(String reply, String prefix, float divisor) {
        if (reply == null || !reply.startsWith(prefix) || !reply.endsWith(";")) return Float.NaN;
        String body = reply.substring(prefix.length(), reply.length() - 1).trim();
        if (body.isEmpty()) return Float.NaN;
        try {
            if (body.indexOf('.') >= 0) {
                return Float.parseFloat(body);
            }
            return Integer.parseInt(body) / divisor;
        } catch (NumberFormatException e) {
            return Float.NaN;
        }
    }
}
