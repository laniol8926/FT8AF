import { useEffect, useState } from "react";
import { api } from "../ipc";
import { SettingsRow } from "./SettingsPrimitives";

/**
 * Logging & Awards, ported from Android's `LoggingSettings.kt`. Priority
 * order per the user: LoTW, QRZ, Cloudlog (World Radio League explicitly
 * not requested). LoTW is real end-to-end here -- confirmations download
 * (`lotw.rs`, matching Android's own `LotwClient.kt`) plus a new,
 * desktop-only upload path via the locally installed TQSL CLI, which
 * Android has no equivalent for (LoTW requires a local private-key
 * signature with no network API for that, and a phone has no TQSL). QRZ
 * and Cloudlog are the next two rows, still disclosed disabled stubs
 * pending their own build pass.
 *
 * Android's "Awards" half of this category's name is desktop's separate
 * Logbook/Stats screen (already built in an earlier phase) -- nothing in
 * this file.
 */
export function LoggingSettings({ callsign }: { callsign: string }) {
  const [lotwOpen, setLotwOpen] = useState(false);
  const [lotwUsername, setLotwUsername] = useState("");

  useEffect(() => {
    api.getConfig("lotwUsername").then((v) => setLotwUsername(v ?? ""));
  }, [lotwOpen]);

  return (
    <>
      {lotwOpen && <LotwDialog callsign={callsign} onClose={() => setLotwOpen(false)} />}
      <div className="settings-category-list panel settings-category-card">
        <SettingsRow
          label="LoTW"
          description="Download QSL confirmations and upload your log via TQSL"
          value={lotwUsername || "Not configured"}
          onClick={() => setLotwOpen(true)}
        />
        <SettingsRow label="QRZ.com" description="Auto-upload QSOs to QRZ Logbook + profile lookups" disabled />
        <SettingsRow label="Cloudlog" description="Upload QSOs to a Cloudlog/Wavelog/Nextlog server" disabled />
      </div>
    </>
  );
}

function LotwDialog({ callsign, onClose }: { callsign: string; onClose: () => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<{ text: string; ok: boolean } | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [confirmUpload, setConfirmUpload] = useState(false);

  useEffect(() => {
    api.getConfig("lotwUsername").then((v) => setUsername(v ?? ""));
    api.getConfig("lotwPassword").then((v) => setPassword(v ?? ""));
    api.lotwPendingUploadCount().then(setPendingCount);
  }, []);

  function save() {
    api.setConfig("lotwUsername", username.trim());
    api.setConfig("lotwPassword", password);
  }

  async function download(fullRefresh: boolean) {
    save();
    setBusy(true);
    setStatus(null);
    const result = await api.lotwDownloadConfirmations(fullRefresh);
    setBusy(false);
    setStatus(
      result.error
        ? { text: result.error, ok: false }
        : { text: `${result.newly_confirmed} QSOs newly confirmed (${result.in_report} in LoTW report)`, ok: true }
    );
  }

  async function upload() {
    setConfirmUpload(false);
    setBusy(true);
    setStatus(null);
    const result = await api.lotwUploadPending(callsign);
    setBusy(false);
    api.lotwPendingUploadCount().then(setPendingCount);
    setStatus(
      result.error
        ? { text: result.error, ok: false }
        : { text: `Uploaded ${result.attempted} QSOs to LoTW via TQSL`, ok: true }
    );
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>LoTW</h3>
          <button className="modal-close" aria-label="Close" onClick={onClose}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Your lotw.arrl.org website login. Downloads your LoTW confirmations and marks
          matching QSOs confirmed. Uploading still needs TQSL (it signs each QSO with your
          certificate) -- this shells out to the TQSL CLI already installed on this
          computer rather than reimplementing LoTW's signing.
        </div>
        <div className="field">
          <label>Username</label>
          <input value={username} onChange={(e) => setUsername(e.target.value)} onBlur={save} autoComplete="off" />
        </div>
        <div className="field">
          <label>Password</label>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            onBlur={save}
            autoComplete="off"
          />
        </div>
        <div className="row" style={{ gap: 8, marginTop: 10 }}>
          <button disabled={busy} onClick={() => download(false)}>
            {busy ? "Working…" : "Download new"}
          </button>
          <button disabled={busy} onClick={() => download(true)}>Download all</button>
        </div>
        {status && (
          <div
            className="muted"
            style={{ marginTop: 8, fontSize: 12, color: status.ok ? "var(--ok)" : "var(--tx)" }}
          >
            {status.text}
          </div>
        )}
        <div style={{ marginTop: 16, paddingTop: 12, borderTop: "1px solid var(--border)" }}>
          <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded to LoTW.`}
          </div>
          {!confirmUpload ? (
            <button disabled={busy || !pendingCount} onClick={() => setConfirmUpload(true)}>
              Upload to LoTW…
            </button>
          ) : (
            <div className="col" style={{ gap: 6 }}>
              <div className="muted" style={{ fontSize: 12 }}>
                This signs and uploads {pendingCount} QSOs to ARRL's real LoTW system using
                your TQSL certificate for {callsign || "your callsign"}. Confirm to proceed.
              </div>
              <div className="row" style={{ gap: 8 }}>
                <button onClick={() => setConfirmUpload(false)}>Cancel</button>
                <button className="primary" disabled={busy} onClick={upload}>
                  {busy ? "Uploading…" : "Confirm upload"}
                </button>
              </div>
            </div>
          )}
        </div>
        <div className="actions">
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
