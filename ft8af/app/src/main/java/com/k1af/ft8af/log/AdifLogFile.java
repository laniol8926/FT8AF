package com.k1af.ft8af.log;

import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.provider.MediaStore;
import android.util.Log;

import com.k1af.ft8af.GeneralVariables;
import com.k1af.ft8af.rigs.BaseRigOperation;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;

/**
 * Maintains a running, append-only ADIF file ({@code ft8af_log.adi}) that receives one
 * {@code <eor>} record the moment each QSO is logged. This gives a crash-proof mirror of
 * {@code QSLTable} independent of the SQLite DB, and lets a desktop logger (LOG4OM, N1MM,
 * DXKeeper, ...) tail the file and import new contacts in real time.
 *
 * <p>The file lives in the app's external files dir alongside {@code debug.log}
 * ({@code /sdcard/Android/data/radio.ks3ckc.ft8af/files/ft8af_log.adi}), so it is reachable
 * over MTP/adb with no runtime storage permission.
 *
 * <p>Robustness: appends are serialized (a single writer lock), the header + {@code <eoh>}
 * is written exactly once on a fresh/empty file (and re-created if the file is deleted out
 * from under the app), every record is flushed immediately so a watching logger sees it and
 * a crash can't lose the last QSO, and any failure is swallowed — appending must never break
 * QSO logging.
 */
public final class AdifLogFile {

    private static final String TAG = "AdifLogFile";
    /** File name in the app's external files dir. */
    public static final String FILE_NAME = "ft8af_log.adi";

    /** Serializes concurrent appends (web logger + on-air completing at the same instant). */
    private static final Object WRITE_LOCK = new Object();

    private AdifLogFile() {}

    /**
     * Append {@code record} for a freshly-logged QSO to the running ADIF file, gated on the
     * {@link GeneralVariables#enableAdifExport} setting. Resolves the file from the app's
     * external files dir. Never throws — a full disk or missing SD must not break logging.
     */
    public static void logQso(Context context, QSLRecord record) {
        if (!GeneralVariables.enableAdifExport) {
            return;
        }
        if (context == null) {
            context = GeneralVariables.getMainContext();
        }
        if (context == null || record == null) {
            return;
        }
        try {
            File dir = context.getExternalFilesDir(null);
            if (dir == null) {
                return;
            }
            File file = new File(dir, FILE_NAME);
            appendRecord(file, fromQslRecord(record));
            // The app's own files dir isn't browsable on Android 11+, so keep a copy where
            // the operator can find it: Download/FT8AF, next to the exports.
            mirrorToDownloads(context, file);
        } catch (Throwable t) {
            // Never let ADIF mirroring break QSO logging. Log the full cause (throwable
            // overload) so storage/IO issues are diagnosable in the field.
            Log.e(TAG, "logQso failed", t);
        }
    }

    /**
     * Map a {@link QSLRecord} to the ADIF field values, matching exactly what the DB stores
     * (and therefore what the bulk {@link ShareLogs} export would later emit for the same row).
     * These are on-air / web-logged real QSOs, so they are never SWL records.
     */
    public static AdifRecord fromQslRecord(QSLRecord record) {
        return new AdifRecord()
                .call(record.getToCallsign())
                .swl(false)
                .lotwQsl(record.isLotW_QSL)
                .manualQsl(record.isQSL)
                .gridsquare(record.getToMaidenGrid())
                .mode(record.getMode())
                .rstSent(AdifFormat.formatReport(record.getMode(), record.getSendReport()))
                .rstRcvd(AdifFormat.formatReport(record.getMode(), record.getReceivedReport()))
                .qsoDate(record.getQso_date())
                .timeOn(record.getTime_on())
                .qsoDateOff(record.getQso_date_off())
                .timeOff(record.getTime_off())
                .band(record.getBandLength())
                .freq(BaseRigOperation.getFrequencyFloat(record.getBandFreq()))
                .stationCallsign(record.getMyCallsign())
                .myGridsquare(record.getMyMaidenGrid())
                .mySig(record.getMySig())
                .mySigInfo(record.getMySigInfo())
                .sig(record.getSig())
                .sigInfo(record.getSigInfo())
                .txPwr(record.getTxPwr())
                .comment(record.getComment());
    }

    /**
     * Copy the whole running log over {@code Download/FT8AF/ft8af_log.adi}, creating it the
     * first time. A full copy (not an append) so the public file always matches the
     * running one, whatever state it was left in. On Android 10+ through MediaStore: the
     * entry this app created is found by name and path and rewritten ("wt"). If it was
     * created by an earlier install (not ours to write), MediaStore makes a numbered copy
     * instead. Never throws.
     */
    static void mirrorToDownloads(Context context, File source) {
        synchronized (WRITE_LOCK) {
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    ContentResolver resolver = context.getContentResolver();
                    Uri collection = MediaStore.Downloads.EXTERNAL_CONTENT_URI;
                    String relativePath = Environment.DIRECTORY_DOWNLOADS + "/FT8AF/";
                    Uri target = null;
                    try (Cursor c = resolver.query(collection,
                            new String[]{MediaStore.Downloads._ID},
                            MediaStore.Downloads.DISPLAY_NAME + "=? AND "
                                    + MediaStore.Downloads.RELATIVE_PATH + "=?",
                            new String[]{FILE_NAME, relativePath}, null)) {
                        if (c != null && c.moveToFirst()) {
                            target = ContentUris.withAppendedId(collection, c.getLong(0));
                        }
                    }
                    if (target == null) {
                        ContentValues values = new ContentValues();
                        values.put(MediaStore.Downloads.DISPLAY_NAME, FILE_NAME);
                        values.put(MediaStore.Downloads.MIME_TYPE, "application/octet-stream");
                        values.put(MediaStore.Downloads.RELATIVE_PATH, relativePath);
                        target = resolver.insert(collection, values);
                    }
                    if (target == null) return;
                    try (OutputStream os = resolver.openOutputStream(target, "wt");
                         FileInputStream in = new FileInputStream(source)) {
                        if (os != null) copy(in, os);
                    }
                } else {
                    File dir = new File(Environment.getExternalStoragePublicDirectory(
                            Environment.DIRECTORY_DOWNLOADS), "FT8AF");
                    //noinspection ResultOfMethodCallIgnored
                    dir.mkdirs();
                    try (FileInputStream in = new FileInputStream(source);
                         FileOutputStream os = new FileOutputStream(new File(dir, FILE_NAME))) {
                        copy(in, os);
                    }
                }
            } catch (Throwable t) {
                Log.e(TAG, "mirrorToDownloads failed", t);
            }
        }
    }

    private static void copy(java.io.InputStream in, OutputStream out) throws IOException {
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
        out.flush();
    }

    /**
     * Append one built ADIF record to {@code file}, writing the header + {@code <eoh>} first
     * iff the file is new or empty. Serialized and flushed. This is the testable core; it
     * works on a plain {@link File} with no Android dependencies.
     */
    public static void appendRecord(File file, AdifRecord record) throws IOException {
        synchronized (WRITE_LOCK) {
            try (FileOutputStream out = new FileOutputStream(file, true)) {
                // Decide header emission from the size of the *opened* stream's file, not a
                // pre-open File.length() check: the file could be deleted between that check
                // and the open (append mode then silently re-creates an empty file), which
                // would leave a headerless ADIF. The channel size reflects the real on-disk
                // state we're about to append to (0 → new/empty/re-created → (re)emit header).
                if (out.getChannel().size() == 0) {
                    out.write(AdifRecord.HEADER.getBytes(StandardCharsets.UTF_8));
                }
                out.write(record.toBytes());
                out.flush();
            }
        }
    }
}
