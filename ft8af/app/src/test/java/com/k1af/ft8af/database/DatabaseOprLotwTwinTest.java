package com.k1af.ft8af.database;

import static com.google.common.truth.Truth.assertThat;

import android.database.Cursor;

import androidx.test.core.app.ApplicationProvider;

import com.k1af.ft8af.log.QSLRecord;

import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;

import java.util.HashMap;

/**
 * Fork: a QSO kept twice in the source logs -- the full record and a LoTW-sourced copy
 * (no RST_RCVD, no FREQ, TIME_OFF = TIME_ON, a few minutes off) -- imports as one QSO in
 * either order, while a LoTW-style record with no full twin is still a QSO of its own.
 * Records are shaped after the AI5II QRZ.com export (K3JOS 2019-06-17, NG4C 2017-11-01,
 * UT5MB 2024-06-07).
 */
@RunWith(RobolectricTestRunner.class)
public class DatabaseOprLotwTwinTest {

    private DatabaseOpr opr;

    @Before
    public void setUp() {
        opr = new DatabaseOpr(ApplicationProvider.getApplicationContext(), null, null,
                DatabaseOpr.SCHEMA_VERSION);
    }

    private static QSLRecord adif(String... kv) {
        HashMap<String, String> map = new HashMap<>();
        for (int i = 0; i < kv.length; i += 2) map.put(kv[i], kv[i + 1]);
        return new QSLRecord(map);
    }

    private static QSLRecord k3josFull() {
        return adif("CALL", "K3JOS", "QSO_DATE", "20190617", "TIME_ON", "0113", "TIME_OFF", "0114",
                "BAND", "40m", "FREQ", "7.07645", "MODE", "FT8", "RST_SENT", "-06", "RST_RCVD", "-21");
    }

    private static QSLRecord k3josLotwCopy() {
        return adif("CALL", "K3JOS", "QSO_DATE", "20190617", "TIME_ON", "0114", "TIME_OFF", "0114",
                "BAND", "40m", "MODE", "FT8", "RST_SENT", "-21", "QSL_RCVD", "Y");
    }

    private int rows(String call) {
        try (Cursor c = opr.getDb().rawQuery("select count(*) from QSLTable where call=?",
                new String[]{call})) {
            c.moveToFirst();
            return c.getInt(0);
        }
    }

    private String field(String call, String column) {
        try (Cursor c = opr.getDb().rawQuery("select " + column + " from QSLTable where call=?",
                new String[]{call})) {
            c.moveToFirst();
            return c.getString(0);
        }
    }

    @Test
    public void lotwCopyAfterFullRecord_mergesIntoIt_andCarriesItsConfirmation() {
        opr.doInsertQSLData(k3josFull(), null, false);
        opr.doInsertQSLData(k3josLotwCopy(), null, false);

        assertThat(rows("K3JOS")).isEqualTo(1);
        assertThat(field("K3JOS", "time_on")).isEqualTo("0113");
        // The copy's RST_SENT (-21, really the report received) must not overwrite ours.
        assertThat(field("K3JOS", "rst_sent")).isEqualTo("-06");
        assertThat(field("K3JOS", "rst_rcvd")).isEqualTo("-21");
        assertThat(field("K3JOS", "isLotW_QSL")).isEqualTo("1");
    }

    @Test
    public void lotwCopyInTheSameMinute_keepsTheFullRecordsReports() {
        opr.doInsertQSLData(k3josFull(), null, false);
        opr.doInsertQSLData(adif("CALL", "K3JOS", "QSO_DATE", "20190617", "TIME_ON", "0113",
                "TIME_OFF", "0113", "BAND", "40m", "MODE", "FT8", "RST_SENT", "-21"), null, false);

        assertThat(rows("K3JOS")).isEqualTo(1);
        assertThat(field("K3JOS", "rst_sent")).isEqualTo("-06");
    }

    @Test
    public void fullRecordAfterLotwCopy_fillsTheCopyInsteadOfAddingARow() {
        opr.doInsertQSLData(k3josLotwCopy(), null, false);
        opr.doInsertQSLData(k3josFull(), null, false);

        assertThat(rows("K3JOS")).isEqualTo(1);
        assertThat(field("K3JOS", "time_on")).isEqualTo("0113");
        assertThat(field("K3JOS", "rst_sent")).isEqualTo("-06");
        assertThat(field("K3JOS", "rst_rcvd")).isEqualTo("-21");
        assertThat(Double.parseDouble(field("K3JOS", "freq"))).isWithin(1e-6).of(7.07645);
        assertThat(field("K3JOS", "isLotW_QSL")).isEqualTo("1");
    }

    @Test
    public void lotwCopyFiveMinutesOff_stillMerges() {
        // NG4C: full 0352-0354, LoTW copy 0347.
        opr.doInsertQSLData(adif("CALL", "NG4C", "QSO_DATE", "20171101", "TIME_ON", "0352",
                "TIME_OFF", "0354", "BAND", "80m", "FREQ", "3.5739", "MODE", "FT8",
                "RST_SENT", "-02", "RST_RCVD", "-24"), null, false);
        opr.doInsertQSLData(adif("CALL", "NG4C", "QSO_DATE", "20171101", "TIME_ON", "0347",
                "TIME_OFF", "0347", "BAND", "80m", "MODE", "FT8", "RST_SENT", "-24"), null, false);

        assertThat(rows("NG4C")).isEqualTo(1);
        assertThat(field("NG4C", "time_on")).isEqualTo("0352");
    }

    @Test
    public void lotwStyleRecordWithNoTwin_isItsOwnQso() {
        // UT5MB: confirmed everywhere, no reports anywhere -- a real QSO.
        opr.doInsertQSLData(adif("CALL", "UT5MB", "QSO_DATE", "20240607", "TIME_ON", "0254",
                "TIME_OFF", "0254", "BAND", "20M", "FREQ", "14.081", "MODE", "FT4"), null, false);

        assertThat(rows("UT5MB")).isEqualTo(1);
    }

    @Test
    public void lotwCopyOnAnotherModeOrTooFarAway_isNotMerged() {
        opr.doInsertQSLData(k3josFull(), null, false);
        opr.doInsertQSLData(adif("CALL", "K3JOS", "QSO_DATE", "20190617", "TIME_ON", "0116",
                "TIME_OFF", "0116", "BAND", "40m", "MODE", "FT4"), null, false);
        opr.doInsertQSLData(adif("CALL", "K3JOS", "QSO_DATE", "20190617", "TIME_ON", "0140",
                "TIME_OFF", "0140", "BAND", "40m", "MODE", "FT8"), null, false);

        assertThat(rows("K3JOS")).isEqualTo(3);
    }

    @Test
    public void twoFullRecordsMinutesApart_stayTwoQsos() {
        // Only a LoTW-style record joins a twin; two real QSOs a few minutes apart don't.
        opr.doInsertQSLData(k3josFull(), null, false);
        opr.doInsertQSLData(adif("CALL", "K3JOS", "QSO_DATE", "20190617", "TIME_ON", "0120",
                "TIME_OFF", "0121", "BAND", "40m", "FREQ", "7.0745", "MODE", "FT8",
                "RST_SENT", "-10", "RST_RCVD", "-12"), null, false);

        assertThat(rows("K3JOS")).isEqualTo(2);
    }

    @Test
    public void noReportSentinelsDisplayBlank() {
        assertThat(DatabaseOpr.displayReport("-120")).isEmpty();
        assertThat(DatabaseOpr.displayReport("-100")).isEmpty();
        assertThat(DatabaseOpr.displayReport(null)).isEmpty();
        assertThat(DatabaseOpr.displayReport("-21")).isEqualTo("-21");
    }
}
