import { useEffect, useState } from "react";
import { api } from "../ipc";
import { SettingsRow } from "./SettingsPrimitives";

/**
 * Logging & Awards, ported from Android's `LoggingSettings.kt`. Priority
 * order per the user: LoTW, QRZ, Cloudlog (World Radio League explicitly
 * not requested).
 *
 * LoTW is real end-to-end: confirmations download (`lotw.rs`, matching
 * Android's own `LotwClient.kt`) plus a new, desktop-only upload path via
 * the locally installed TQSL CLI, which Android has no equivalent for
 * (LoTW requires a local private-key signature with no network API for
 * that, and a phone has no TQSL).
 *
 * QRZ is also real end-to-end now (`qrz.rs`), split into Android's own
 * real two-row structure (confirmed against `LoggingSettings.kt`, not
 * guessed): a "QRZ.com" row with an inline enable toggle + the Logbook
 * API key dialog (upload credential), and a separate "QRZ Profile Lookup"
 * row for the XML username/password (lookup credential) -- these are two
 * different QRZ subscriptions and Android keeps them as two different
 * rows/dialogs for exactly that reason. Station lookup during a QSO
 * (state/grid/country, backing a new logging-time STATE/COUNTRY
 * backfill) and auto-upload to the Logbook API the moment a QSO is
 * logged are both wired for real; see `qrz.rs`'s top doc comment. Not
 * ported: the profile-photo half of Android's XML lookup (no per-contact
 * avatar UI exists anywhere on desktop to show it) -- the Profile Lookup
 * row's description is adapted accordingly rather than copying Android's
 * "avatars" wording verbatim, since that part genuinely isn't built.
 *
 * **Manual upload placement, a deliberate desktop-only difference**:
 * Android's equivalent (`ThirdPartyService.syncAllQSOs`) is triggered
 * from the Logbook screen, not from either of these Settings dialogs --
 * desktop doesn't have that screen's sync button yet, so the Logbook
 * dialog below carries its own "Upload to QRZ now" instead of leaving the
 * feature with no manual trigger at all.
 *
 * Cloudlog stays a disclosed disabled stub pending its own build pass.
 *
 * Android's "Awards" half of this category's name is desktop's separate
 * Logbook/Stats screen (already built in an earlier phase) -- nothing in
 * this file.
 */
export function LoggingSettings({ callsign }: { callsign: string }) {
  const [lotwOpen, setLotwOpen] = useState(false);
  const [lotwUsername, setLotwUsername] = useState("");
  const [qrzLogbookOpen, setQrzLogbookOpen] = useState(false);
  const [qrzCredsOpen, setQrzCredsOpen] = useState(false);
  const [qrzEnabled, setQrzEnabled] = useState(false);
  const [qrzApiKeySet, setQrzApiKeySet] = useState(false);
  const [qrzXmlUser, setQrzXmlUser] = useState("");
  const [qrzXmlPassSet, setQrzXmlPassSet] = useState(false);

  useEffect(() => {
    api.getConfig("lotwUsername").then((v) => setLotwUsername(v ?? ""));
  }, [lotwOpen]);

  function refreshQrz() {
    api.getConfig("enableQRZ").then((v) => setQrzEnabled(v === "1"));
    api.getConfig("qrzApiKey").then((v) => setQrzApiKeySet(!!v));
    api.getConfig("qrzXmlUsername").then((v) => setQrzXmlUser(v ?? ""));
    api.getConfig("qrzXmlPassword").then((v) => setQrzXmlPassSet(!!v));
  }
  useEffect(refreshQrz, [qrzLogbookOpen, qrzCredsOpen]);

  function saveQrzEnabled(next: boolean) {
    setQrzEnabled(next);
    api.setConfig("enableQRZ", next ? "1" : "0");
  }

  return (
    <>
      {lotwOpen && <LotwDialog callsign={callsign} onClose={() => setLotwOpen(false)} />}
      {qrzLogbookOpen && <QrzLogbookDialog onClose={() => setQrzLogbookOpen(false)} />}
      {qrzCredsOpen && <QrzCredsDialog onClose={() => setQrzCredsOpen(false)} />}
      <div className="settings-category-list panel settings-category-card">
        <SettingsRow
          label="LoTW"
          description="Download QSL confirmations and upload your log via TQSL"
          value={lotwUsername || "Not configured"}
          onClick={() => setLotwOpen(true)}
        />
        <SettingsRow
          label="QRZ.com"
          description="Auto-upload QSOs to QRZ Logbook (requires a Logbook API key)"
          value={qrzApiKeySet ? "Configured" : "Not configured"}
          toggle={qrzEnabled}
          onToggleChange={saveQrzEnabled}
          onClick={() => setQrzLogbookOpen(true)}
        />
        <SettingsRow
          label="QRZ Profile Lookup"
          description="Username + password for the QRZ XML API (state/grid/country during a QSO)"
          value={qrzXmlUser && qrzXmlPassSet ? qrzXmlUser : "Not configured"}
          onClick={() => setQrzCredsOpen(true)}
        />
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

/**
 * QRZ Logbook API key -- the upload credential. Matches Android's
 * `QrzLogbookDialog`: a single key field, a read-only "Test Connection"
 * (`ACTION=STATUS`, inserts nothing) showing Pass/Fail, Cancel/Save.
 * Desktop adds one thing Android's dialog doesn't have: a manual "Upload
 * to QRZ now" button -- see this file's top doc comment for why.
 *
 * **Restore-on-cancel, a deliberate desktop adaptation**: Android tests
 * against an in-memory `GeneralVariables` cache and only its own Save
 * button writes the real persisted DB row, so Test can safely mutate that
 * cache and a Cancel just drops it. Desktop's `config` table has no such
 * separate scratch layer -- `setConfig` IS the persisted value -- so
 * Test here writes it directly (the backend command reads config, not a
 * typed-but-unsaved value) and Cancel/X restores the original persisted
 * key, mirroring Android's own `handleDismiss` restore intent instead of
 * leaving a tested-but-never-explicitly-saved key sitting in storage.
 */
function QrzLogbookDialog({ onClose }: { onClose: () => void }) {
  const [initialApiKey, setInitialApiKey] = useState<string | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    api.getConfig("qrzApiKey").then((v) => {
      setInitialApiKey(v ?? "");
      setApiKey(v ?? "");
    });
    api.qrzPendingUploadCount().then(setPendingCount);
  }, []);

  function dismiss() {
    if (initialApiKey !== null) api.setConfig("qrzApiKey", initialApiKey);
    onClose();
  }

  async function test() {
    await api.setConfig("qrzApiKey", apiKey.trim());
    setBusy(true);
    setTestResult(null);
    try {
      await api.qrzTestLogbookConnection();
      setTestResult({ text: "Pass", ok: true });
    } catch (e) {
      setTestResult({ text: String(e), ok: false });
    }
    setBusy(false);
  }

  async function save() {
    await api.setConfig("qrzApiKey", apiKey.trim());
    onClose();
  }

  async function upload() {
    setBusy(true);
    setUploadStatus(null);
    const result = await api.qrzUploadPending();
    setBusy(false);
    api.qrzPendingUploadCount().then(setPendingCount);
    if (result.error) {
      setUploadStatus({
        text: `${result.uploaded} uploaded, ${result.duplicates} already on QRZ, ${result.failed} failed -- ${result.error}`,
        ok: result.uploaded + result.duplicates > 0,
      });
    } else {
      setUploadStatus({ text: `${result.uploaded} uploaded, ${result.duplicates} already on QRZ`, ok: true });
    }
  }

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>QRZ Logbook</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Logbook API key from QRZ Logbook → Settings (not your account password).
        </div>
        <div className="field">
          <label>API Key</label>
          <input
            value={apiKey}
            onChange={(e) => {
              setApiKey(e.target.value);
              setTestResult(null);
            }}
            placeholder="Your QRZ Logbook API key"
            autoComplete="off"
          />
        </div>
        <div className="row" style={{ gap: 8, marginTop: 10, alignItems: "center" }}>
          <button disabled={busy} onClick={test}>{busy ? "Working…" : "Test Connection"}</button>
          {testResult && (
            <span style={{ fontSize: 13, fontWeight: 600, color: testResult.ok ? "var(--ok)" : "var(--tx)" }}>
              {testResult.text}
            </span>
          )}
        </div>

        <div style={{ marginTop: 16, paddingTop: 12, borderTop: "1px solid var(--border)" }}>
          <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded to QRZ.`}
          </div>
          <button disabled={busy || !pendingCount} onClick={upload}>
            {busy ? "Uploading…" : "Upload to QRZ now"}
          </button>
          {uploadStatus && (
            <div className="muted" style={{ marginTop: 8, fontSize: 12, color: uploadStatus.ok ? "var(--ok)" : "var(--tx)" }}>
              {uploadStatus.text}
            </div>
          )}
        </div>

        <div className="actions">
          <button onClick={dismiss}>Cancel</button>
          <button className="primary" onClick={save}>Save</button>
        </div>
      </div>
    </div>
  );
}

/**
 * QRZ XML API username/password -- the lookup credential (separate paid
 * subscription from the free Logbook API key). Matches Android's
 * `QrzCredsDialog`: username + password, "Test Connection" showing
 * Pass/the raw error text, Cancel/Save. Restore-on-cancel for the same
 * reason as `QrzLogbookDialog` above.
 */
function QrzCredsDialog({ onClose }: { onClose: () => void }) {
  const [initial, setInitial] = useState<{ user: string; pass: string } | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    Promise.all([api.getConfig("qrzXmlUsername"), api.getConfig("qrzXmlPassword")]).then(([u, p]) => {
      setInitial({ user: u ?? "", pass: p ?? "" });
      setUsername(u ?? "");
      setPassword(p ?? "");
    });
  }, []);

  function dismiss() {
    if (initial) {
      api.setConfig("qrzXmlUsername", initial.user);
      api.setConfig("qrzXmlPassword", initial.pass);
    }
    onClose();
  }

  async function test() {
    await Promise.all([
      api.setConfig("qrzXmlUsername", username.trim()),
      api.setConfig("qrzXmlPassword", password),
    ]);
    setBusy(true);
    setTestResult(null);
    const result = await api.qrzTestXmlConnection();
    setTestResult(result === "OK" ? { text: "Pass", ok: true } : { text: result, ok: false });
    setBusy(false);
  }

  async function save() {
    await Promise.all([
      api.setConfig("qrzXmlUsername", username.trim()),
      api.setConfig("qrzXmlPassword", password),
    ]);
    onClose();
  }

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>QRZ Profile Lookup</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Used to look up a station's state/grid/country while you're in a QSO with them,
          so it can be logged automatically. Requires a QRZ XML subscription.
        </div>
        <div className="field">
          <label>Username</label>
          <input
            value={username}
            onChange={(e) => {
              setUsername(e.target.value);
              setTestResult(null);
            }}
            autoComplete="off"
          />
        </div>
        <div className="field">
          <label>Password</label>
          <input
            type="password"
            value={password}
            onChange={(e) => {
              setPassword(e.target.value);
              setTestResult(null);
            }}
            autoComplete="off"
          />
        </div>
        <div className="row" style={{ gap: 8, marginTop: 10, alignItems: "center" }}>
          <button disabled={busy} onClick={test}>{busy ? "Working…" : "Test Connection"}</button>
          {testResult && (
            <span style={{ fontSize: 13, fontWeight: 600, color: testResult.ok ? "var(--ok)" : "var(--tx)" }}>
              {testResult.text}
            </span>
          )}
        </div>
        <div className="actions">
          <button onClick={dismiss}>Cancel</button>
          <button className="primary" onClick={save}>Save</button>
        </div>
      </div>
    </div>
  );
}
