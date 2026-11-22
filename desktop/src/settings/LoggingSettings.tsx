import { useEffect, useState } from "react";
import { api } from "../ipc";
import type { StationProfile, UdpConfig } from "../ipc";
import { ListPickerDialog, SettingsRow, SettingsSection } from "./SettingsPrimitives";

/**
 * Logging & Awards, ported from Android's `LoggingSettings.kt`. Built in
 * priority order per the user, across two sessions: LoTW, QRZ first; then
 * Cloudlog/Wavelog/Nextlog and World Radio League once the user asked for
 * them too ("others use them" even though they personally don't).
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
 * Cloudlog/Wavelog/Nextlog (`cloudlog.rs`) and World Radio League
 * (`wrl.rs`) are both real end-to-end now too, requested directly by the
 * user ("others use them" -- they don't personally, so neither could be
 * live-verified against a real server/account the way QRZ/LoTW were).
 * Each gets the same shape as QRZ: an enable toggle + address/key dialog,
 * test connection, and a manual catch-up upload alongside the automatic
 * upload-on-log. Cloudlog is **upload only** -- Android also has an
 * optional two-way sync that downloads QSOs from the Wavelog logbook and
 * auto-inserts the ones missing locally (issue #965 there); the user
 * explicitly chose not to port that half, since it inserts new QSO rows
 * from a remote server and they have no way to verify it live. WRL needs
 * a destination logbook before it will accept any contact -- a brand-new
 * account has none, so its dialog also offers to create one.
 *
 * "Save SWL Decodes"/"Save SWL QSOs" (logging stations merely *heard*, no
 * completed two-way QSO) stay disclosed disabled stubs -- needs a new SWL
 * DB schema this project's own notes already defer to the later "QSO
 * logic replacement" effort.
 *
 * PSKReporter's row gates *uploading your own decoded spots* via
 * Android's IPFIX/UDP `PskReporterSender.kt` -- a different, unbuilt
 * protocol, NOT the Map screen's already-working "who's hearing me"
 * overlay that reuses this same `psk_reporter.rs` module to fetch
 * (confirmed live). The uploader itself isn't built, but the toggle IS
 * real and persists for real (per the user's explicit ask -- "there
 * should be a toggle... and it is not there"), matching this project's
 * established disclosed-but-real-preference pattern (e.g. Transmission's
 * ALC Auto-Volume): nothing reads `enablePskReporter` yet, but the
 * operator's choice is never silently discarded just because the backend
 * isn't wired up.
 *
 * The "ADIF log file" row is real (`adif_log_file.rs`): every logged QSO
 * is appended to a running `ft8af_log.adi` the moment it's logged, for a
 * desktop logger (LOG4OM, N1MM, DXKeeper) to tail. Written straight to
 * `~/Downloads/FT8AF/` -- simpler than Android's app-private-file-plus-
 * MediaStore-mirror dance, which solves a scoped-storage problem desktop
 * doesn't have.
 *
 * The "WSJT-X UDP" section is real and moved here from Radio & Audio to
 * match Android's actual placement (confirmed: it's a `LoggingSettings.kt`
 * section, not a Radio & Audio one) -- same `set_udp_config`/`udp_*`
 * config keys as before, just relocated, not rebuilt.
 *
 * Android's "Awards" half of this category's name is desktop's separate
 * Logbook/Stats screen (already built in an earlier phase) -- nothing in
 * this file.
 *
 * **"Mark QSOs as Uploaded to All Logs"**, new, no Android equivalent:
 * requested directly by the user after noticing this app's own QRZ
 * pending-upload retry (added the same session) would otherwise keep
 * trying to re-upload QSOs a station had already gotten into QRZ.com
 * (or any other log) some other way -- importing an ADIF file directly on
 * that service's own website doesn't touch this app's `synced_*`/
 * `lotw_sent` columns at all, so from this app's point of view those QSOs
 * still look pending forever. A blunt "stop trying, I already handled it"
 * action (`db.rs`'s `mark_all_synced`): flips every currently-unsynced
 * QSO's QRZ/Cloudlog/LoTW-sent flags to synced in one sweep, without
 * contacting any of those services.
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
  const [adifExportEnabled, setAdifExportEnabled] = useState(true);
  const [pskReporterEnabled, setPskReporterEnabled] = useState(true);
  const [udpOpen, setUdpOpen] = useState(false);
  const [udp, setUdp] = useState<UdpConfig>({ enabled: false, host: "127.0.0.1", port: 2237, accept_requests: false });
  const [markAllOpen, setMarkAllOpen] = useState(false);
  const [cloudlogOpen, setCloudlogOpen] = useState(false);
  const [cloudlogEnabled, setCloudlogEnabled] = useState(false);
  const [cloudlogAddress, setCloudlogAddress] = useState("");
  const [wrlOpen, setWrlOpen] = useState(false);
  const [wrlEnabled, setWrlEnabled] = useState(false);
  const [wrlApiKeySet, setWrlApiKeySet] = useState(false);
  const [hamqthOpen, setHamqthOpen] = useState(false);
  const [hamqthEnabled, setHamqthEnabled] = useState(false);
  const [hamqthPasswordSet, setHamqthPasswordSet] = useState(false);
  const [clublogOpen, setClublogOpen] = useState(false);
  const [clublogEnabled, setClublogEnabled] = useState(false);
  const [clublogTokenSet, setClublogTokenSet] = useState(false);
  const [hrdlogOpen, setHrdlogOpen] = useState(false);
  const [hrdlogEnabled, setHrdlogEnabled] = useState(false);
  const [hrdlogCodeSet, setHrdlogCodeSet] = useState(false);
  const [eqslOpen, setEqslOpen] = useState(false);
  const [eqslEnabled, setEqslEnabled] = useState(false);
  const [eqslUsername, setEqslUsername] = useState("");

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

  function refreshCloudlog() {
    api.getConfig("enableCloudlog").then((v) => setCloudlogEnabled(v === "1"));
    api.getConfig("cloudlogServerAddress").then((v) => setCloudlogAddress(v ?? ""));
  }
  useEffect(refreshCloudlog, [cloudlogOpen]);
  function saveCloudlogEnabled(next: boolean) {
    setCloudlogEnabled(next);
    api.setConfig("enableCloudlog", next ? "1" : "0");
  }

  function refreshWrl() {
    api.getConfig("enableWRL").then((v) => setWrlEnabled(v === "1"));
    api.getConfig("wrlApiKey").then((v) => setWrlApiKeySet(!!v));
  }
  useEffect(refreshWrl, [wrlOpen]);
  function saveWrlEnabled(next: boolean) {
    setWrlEnabled(next);
    api.setConfig("enableWRL", next ? "1" : "0");
  }

  function refreshHamqth() {
    api.getConfig("enableHamQTH").then((v) => setHamqthEnabled(v === "1"));
    api.getConfig("hamqthPassword").then((v) => setHamqthPasswordSet(!!v));
  }
  useEffect(refreshHamqth, [hamqthOpen]);
  function saveHamqthEnabled(next: boolean) {
    setHamqthEnabled(next);
    api.setConfig("enableHamQTH", next ? "1" : "0");
  }

  function refreshClublog() {
    api.getConfig("enableClublog").then((v) => setClublogEnabled(v === "1"));
    api.getConfig("clublogToken").then((v) => setClublogTokenSet(!!v));
  }
  useEffect(refreshClublog, [clublogOpen]);
  function saveClublogEnabled(next: boolean) {
    setClublogEnabled(next);
    api.setConfig("enableClublog", next ? "1" : "0");
  }

  function refreshHrdlog() {
    api.getConfig("enableHRDLog").then((v) => setHrdlogEnabled(v === "1"));
    api.getConfig("hrdlogCode").then((v) => setHrdlogCodeSet(!!v));
  }
  useEffect(refreshHrdlog, [hrdlogOpen]);
  function saveHrdlogEnabled(next: boolean) {
    setHrdlogEnabled(next);
    api.setConfig("enableHRDLog", next ? "1" : "0");
  }

  function refreshEqsl() {
    api.getConfig("enableEqsl").then((v) => setEqslEnabled(v === "1"));
    api.getConfig("eqslUsername").then((v) => setEqslUsername(v ?? ""));
  }
  useEffect(refreshEqsl, [eqslOpen]);
  function saveEqslEnabled(next: boolean) {
    setEqslEnabled(next);
    api.setConfig("enableEqsl", next ? "1" : "0");
  }

  useEffect(() => {
    // Defaults to on, matching Android -- see adif_log_file.rs's doc
    // comment on why this new-to-desktop feature uses Android's own
    // default rather than this project's usual "preserve prior desktop
    // behavior" rule (there is no prior behavior to preserve).
    api.getConfig("enableAdifExport").then((v) => setAdifExportEnabled(v !== "0"));
  }, []);
  function saveAdifExportEnabled(next: boolean) {
    setAdifExportEnabled(next);
    api.setConfig("enableAdifExport", next ? "1" : "0");
  }

  useEffect(() => {
    // Real, persisted preference (defaults to on, matching Android) even
    // though nothing reads it yet -- the actual IPFIX/UDP sender is still
    // deferred (see this file's top doc comment). Matches this project's
    // established pattern for a disclosed-but-real config toggle (e.g.
    // Transmission's ALC Auto-Volume): the operator's choice is never
    // silently discarded just because the backend isn't wired up.
    api.getConfig("enablePskReporter").then((v) => setPskReporterEnabled(v !== "0"));
  }, []);
  function savePskReporterEnabled(next: boolean) {
    setPskReporterEnabled(next);
    api.setConfig("enablePskReporter", next ? "1" : "0");
  }

  function refreshUdp() {
    Promise.all([
      api.getConfig("udp_enabled"),
      api.getConfig("udp_host"),
      api.getConfig("udp_port"),
      api.getConfig("udp_accept_requests"),
    ]).then(([en, host, port, accept]) => {
      const p = parseInt(port ?? "", 10);
      setUdp((prev) => ({
        enabled: en === "true",
        host: host && host.length > 0 ? host : prev.host,
        port: Number.isFinite(p) && p > 0 && p <= 65535 ? p : prev.port,
        accept_requests: accept === "true",
      }));
    });
  }
  useEffect(refreshUdp, [udpOpen]);
  function applyUdp(next: UdpConfig) {
    setUdp(next);
    api.setUdpConfig(next);
  }

  return (
    <>
      {lotwOpen && <LotwDialog callsign={callsign} onClose={() => setLotwOpen(false)} />}
      {qrzLogbookOpen && <QrzLogbookDialog onClose={() => setQrzLogbookOpen(false)} />}
      {qrzCredsOpen && <QrzCredsDialog onClose={() => setQrzCredsOpen(false)} />}
      {udpOpen && <UdpServerDialog udp={udp} onSave={applyUdp} onClose={() => setUdpOpen(false)} />}
      {markAllOpen && <MarkAllUploadedDialog onClose={() => setMarkAllOpen(false)} />}
      {cloudlogOpen && <CloudlogDialog onClose={() => setCloudlogOpen(false)} />}
      {wrlOpen && <WrlDialog callsign={callsign} onClose={() => setWrlOpen(false)} />}
      {hamqthOpen && <HamqthDialog callsign={callsign} onClose={() => setHamqthOpen(false)} />}
      {clublogOpen && <ClublogDialog onClose={() => setClublogOpen(false)} />}
      {hrdlogOpen && <HrdlogDialog callsign={callsign} onClose={() => setHrdlogOpen(false)} />}
      {eqslOpen && <EqslDialog onClose={() => setEqslOpen(false)} />}
      <SettingsSection title="Logging & Awards" />
      <div className="settings-category-list panel settings-category-card">
        <SettingsRow label="Save SWL Decodes" description="Log all decoded messages to the database" disabled />
        <SettingsRow label="Save SWL QSOs" description="Log QSOs detected between other stations" disabled />
        <SettingsRow
          label="PSKReporter"
          description="Upload decoded spots to pskreporter.info -- preference saved for real, but the uploader itself isn't built yet (separate from the Map screen's spot overlay, which already works)"
          toggle={pskReporterEnabled}
          onToggleChange={savePskReporterEnabled}
        />
        <SettingsRow
          label="ADIF log file"
          description="Append each logged QSO to ft8af_log.adi for backup and desktop-logger (LOG4OM) monitor import. A copy is kept in Downloads/FT8AF."
          toggle={adifExportEnabled}
          onToggleChange={saveAdifExportEnabled}
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
        <SettingsRow
          label="Cloudlog"
          description="Upload QSOs to a Cloudlog/Wavelog/Nextlog server"
          value={cloudlogAddress || "Not configured"}
          toggle={cloudlogEnabled}
          onToggleChange={saveCloudlogEnabled}
          onClick={() => setCloudlogOpen(true)}
        />
        <SettingsRow
          label="World Radio League"
          description="Auto-upload QSOs to your World Radio League logbook (requires a Developer API key)"
          value={wrlApiKeySet ? "Configured" : "Not configured"}
          toggle={wrlEnabled}
          onToggleChange={saveWrlEnabled}
          onClick={() => setWrlOpen(true)}
        />
        <SettingsRow
          label="HamQTH"
          description="Auto-upload QSOs to your HamQTH logbook"
          value={hamqthPasswordSet ? "Configured" : "Not configured"}
          toggle={hamqthEnabled}
          onToggleChange={saveHamqthEnabled}
          onClick={() => setHamqthOpen(true)}
        />
        <SettingsRow
          label="Club Log"
          description="Auto-upload QSOs to Club Log (not to be confused with Cloudlog above)"
          value={clublogTokenSet ? "Configured" : "Not configured"}
          toggle={clublogEnabled}
          onToggleChange={saveClublogEnabled}
          onClick={() => setClublogOpen(true)}
        />
        <SettingsRow
          label="HRDLog.net"
          description="Auto-upload QSOs to HRDLog.net"
          value={hrdlogCodeSet ? "Configured" : "Not configured"}
          toggle={hrdlogEnabled}
          onToggleChange={saveHrdlogEnabled}
          onClick={() => setHrdlogOpen(true)}
        />
        <SettingsRow
          label="eQSL"
          description="Upload QSOs to eQSL and download confirmed QSLs into this log"
          value={eqslUsername || "Not configured"}
          toggle={eqslEnabled}
          onToggleChange={saveEqslEnabled}
          onClick={() => setEqslOpen(true)}
        />
        <SettingsRow
          label="LoTW"
          description="Download QSL confirmations and upload your log via TQSL"
          value={lotwUsername || "Not configured"}
          onClick={() => setLotwOpen(true)}
        />
        <SettingsRow
          label="Mark QSOs as Uploaded to All Logs"
          description="Already got your log into QRZ, Cloudlog, LoTW, World Radio League, HamQTH, or Club Log some other way (e.g. an ADIF import done directly on that service's website)? Use this so this app stops trying to upload those same QSOs again."
          onClick={() => setMarkAllOpen(true)}
        />
      </div>

      <SettingsSection title="WSJT-X UDP" />
      <div className="settings-category-list panel settings-category-card">
        <SettingsRow
          label="Enable UDP broadcast"
          description="Broadcast decodes, status and logged QSOs to companion apps (GridTracker, JTAlert, N1MM, Log4OM)."
          toggle={udp.enabled}
          onToggleChange={(checked) => applyUdp({ ...udp, enabled: checked })}
        />
        <SettingsRow
          label="Server address"
          description="Destination host and port for outgoing datagrams."
          value={`${udp.host}:${udp.port}`}
          onClick={() => setUdpOpen(true)}
        />
        <SettingsRow
          label="Accept UDP requests"
          description="Let companion apps reply to a call, halt TX, or send free text."
          toggle={udp.accept_requests}
          onToggleChange={(checked) => applyUdp({ ...udp, accept_requests: checked })}
        />
      </div>
    </>
  );
}

/** Host/port dialog for the WSJT-X UDP server address, matching Android's
 *  `WsjtxUdpDialog`: Save disabled unless the port parses into 1..65535. */
function UdpServerDialog({
  udp,
  onSave,
  onClose,
}: {
  udp: UdpConfig;
  onSave: (next: UdpConfig) => void;
  onClose: () => void;
}) {
  const [host, setHost] = useState(udp.host);
  const [portText, setPortText] = useState(String(udp.port));
  const port = parseInt(portText, 10);
  const valid = host.trim().length > 0 && Number.isFinite(port) && port >= 1 && port <= 65535;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>WSJT-X UDP server</h3>
          <button className="modal-close" aria-label="Close" onClick={onClose}>✕</button>
        </div>
        <div className="field">
          <label>Host</label>
          <input value={host} onChange={(e) => setHost(e.target.value)} autoComplete="off" />
        </div>
        <div className="field">
          <label>Port</label>
          <input
            value={portText}
            onChange={(e) => setPortText(e.target.value.replace(/\D/g, "").slice(0, 5))}
            inputMode="numeric"
          />
        </div>
        <div className="actions">
          <button onClick={onClose}>Cancel</button>
          <button
            className="primary"
            disabled={!valid}
            onClick={() => {
              onSave({ ...udp, host: host.trim(), port });
              onClose();
            }}
          >
            Save
          </button>
        </div>
      </div>
    </div>
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
 * "Mark QSOs as Uploaded to All Logs" -- see this file's top doc comment.
 * A two-step confirm (same pattern as `LotwDialog`'s upload button) since
 * it affects every pending QSO at once and can't be targeted to just one.
 */
function MarkAllUploadedDialog({ onClose }: { onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [result, setResult] = useState<number | null>(null);

  async function markAll() {
    setConfirming(false);
    setBusy(true);
    const n = await api.markAllQsosUploaded();
    setBusy(false);
    setResult(n);
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>Mark QSOs as Uploaded to All Logs</h3>
          <button className="modal-close" aria-label="Close" onClick={onClose}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Marks every QSO this app thinks is still pending upload as already
          uploaded to QRZ, Cloudlog, LoTW, World Radio League, HamQTH, and
          Club Log -- without actually contacting any of them. Use this if
          you've uploaded your log to one of these
          another way, such as importing an ADIF file directly on that
          service's own website; this app has no way to detect that on its
          own, so it would otherwise keep trying to upload those same QSOs
          again (including automatically on launch for QRZ).
        </div>
        {result == null ? (
          !confirming ? (
            <button disabled={busy} onClick={() => setConfirming(true)}>
              Mark all as uploaded…
            </button>
          ) : (
            <div className="col" style={{ gap: 6 }}>
              <div className="muted" style={{ fontSize: 12 }}>
                This cannot be undone from here -- if any of these QSOs
                genuinely weren't uploaded anywhere, you'd need to re-upload
                them manually afterward. Confirm to proceed.
              </div>
              <div className="row" style={{ gap: 8 }}>
                <button onClick={() => setConfirming(false)}>Cancel</button>
                <button className="primary" disabled={busy} onClick={markAll}>
                  {busy ? "Marking…" : "Confirm"}
                </button>
              </div>
            </div>
          )
        ) : (
          <div className="muted" style={{ fontSize: 12, color: "var(--ok)" }}>
            {result} QSO{result === 1 ? "" : "s"} marked as uploaded.
          </div>
        )}
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

/**
 * Cloudlog/Wavelog/Nextlog settings -- server address, API key, Station ID
 * (`station_profile_id`, a whole number; Android's own real-world lesson,
 * see `cloudlog.rs`'s doc comment, is that a callsign/name there makes
 * every upload fail). Test Connection saves the typed values first (the
 * Rust side reads config directly, matching `QrzLogbookDialog`'s own
 * restore-on-cancel reasoning) and, on success, fetches station profiles
 * for the picker below.
 */
function CloudlogDialog({ onClose }: { onClose: () => void }) {
  const [initial, setInitial] = useState<{ address: string; apiKey: string; stationId: string } | null>(null);
  const [address, setAddress] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [stationId, setStationId] = useState("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);
  const [stations, setStations] = useState<StationProfile[]>([]);
  const [showStationPicker, setShowStationPicker] = useState(false);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    Promise.all([
      api.getConfig("cloudlogServerAddress"),
      api.getConfig("cloudlogApiKey"),
      api.getConfig("cloudlogStationID"),
    ]).then(([a, k, s]) => {
      setInitial({ address: a ?? "", apiKey: k ?? "", stationId: s ?? "" });
      setAddress(a ?? "");
      setApiKey(k ?? "");
      setStationId(s ?? "");
      if ((a ?? "").trim() && (k ?? "").trim()) {
        api.cloudlogFetchStations(a ?? "", k ?? "").then(setStations);
      }
    });
    api.cloudlogPendingUploadCount().then(setPendingCount);
  }, []);

  const stationIdValid = stationId.trim() === "" || /^[0-9]+$/.test(stationId.trim());

  function dismiss() {
    if (initial) {
      api.setConfig("cloudlogServerAddress", initial.address);
      api.setConfig("cloudlogApiKey", initial.apiKey);
      api.setConfig("cloudlogStationID", initial.stationId);
    }
    onClose();
  }

  async function test() {
    await Promise.all([
      api.setConfig("cloudlogServerAddress", address.trim()),
      api.setConfig("cloudlogApiKey", apiKey.trim()),
      api.setConfig("cloudlogStationID", stationId.trim()),
    ]);
    setBusy(true);
    setTestResult(null);
    const check = await api.cloudlogTestConnection();
    setTestResult({ text: check.ok ? "Pass" : check.detail || "Fail", ok: check.ok });
    if (check.ok) {
      const list = await api.cloudlogFetchStations(address.trim(), apiKey.trim());
      setStations(list);
    }
    setBusy(false);
  }

  async function save() {
    await Promise.all([
      api.setConfig("cloudlogServerAddress", address.trim()),
      api.setConfig("cloudlogApiKey", apiKey.trim()),
      api.setConfig("cloudlogStationID", stationId.trim()),
    ]);
    onClose();
  }

  async function upload() {
    setBusy(true);
    setUploadStatus(null);
    const result = await api.cloudlogUploadPending();
    setBusy(false);
    api.cloudlogPendingUploadCount().then(setPendingCount);
    if (result.error) {
      setUploadStatus({
        text: `${result.uploaded} uploaded, ${result.already_present} already there, ${result.failed} failed -- ${result.error}`,
        ok: result.uploaded + result.already_present > 0,
      });
    } else {
      setUploadStatus({ text: `${result.uploaded} uploaded, ${result.already_present} already there`, ok: true });
    }
  }

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>Cloudlog / Wavelog / Nextlog</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Your self-hosted (or hosted) logbook server's address and API key, from its own
          Settings/Integrations page. Upload only -- does not download or import QSOs.
        </div>
        <div className="field">
          <label>Server address</label>
          <input
            value={address}
            onChange={(e) => {
              setAddress(e.target.value);
              setTestResult(null);
              setStations([]);
            }}
            placeholder="https://log.example.com/"
            autoComplete="off"
          />
        </div>
        <div className="field">
          <label>API key</label>
          <input
            value={apiKey}
            onChange={(e) => {
              setApiKey(e.target.value);
              setTestResult(null);
              setStations([]);
            }}
            placeholder="Your Cloudlog/Wavelog API key"
            autoComplete="off"
          />
        </div>
        <div className="field">
          <label>Station ID</label>
          <input
            value={stationId}
            onChange={(e) => setStationId(e.target.value.replace(/[^0-9]/g, ""))}
            placeholder="A whole number, e.g. 1"
            inputMode="numeric"
          />
          {!stationIdValid && (
            <div className="muted" style={{ fontSize: 12, color: "var(--tx)" }}>
              Must be a plain number (your station location's id, not a callsign).
            </div>
          )}
          {stations.length > 0 && (
            <button style={{ marginTop: 6 }} onClick={() => setShowStationPicker(true)}>
              Choose from {stations.length} station{stations.length === 1 ? "" : "s"}…
            </button>
          )}
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
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded.`}
          </div>
          <button disabled={busy || !pendingCount} onClick={upload}>
            {busy ? "Uploading…" : "Upload now"}
          </button>
          {uploadStatus && (
            <div className="muted" style={{ marginTop: 8, fontSize: 12, color: uploadStatus.ok ? "var(--ok)" : "var(--tx)" }}>
              {uploadStatus.text}
            </div>
          )}
        </div>

        <div className="actions">
          <button onClick={dismiss}>Cancel</button>
          <button className="primary" disabled={!stationIdValid} onClick={save}>Save</button>
        </div>
      </div>
      {showStationPicker && (
        <ListPickerDialog
          title="Station profile"
          options={stations.map((s) => ({ value: s.station_id, label: stationLabel(s) }))}
          value={stationId}
          onSelect={(v) => {
            setStationId(v);
            setShowStationPicker(false);
          }}
          onCancel={() => setShowStationPicker(false)}
          searchable
        />
      )}
    </div>
  );
}

function stationLabel(s: StationProfile): string {
  let out = s.station_id;
  if (s.profile_name) out += ` - ${s.profile_name}`;
  if (s.callsign) out += ` (${s.callsign}${s.gridsquare ? `, ${s.gridsquare}` : ""})`;
  return out;
}

/**
 * World Radio League settings -- a Developer API key, no server address
 * (WRL is a fixed cloud service). A fresh account has no logbook at all,
 * so a lone "Create logbook" button appears once the key is entered and
 * the account is confirmed to have none -- matches Android's own
 * `WrlSettingsDialog`.
 */
function WrlDialog({ callsign, onClose }: { callsign: string; onClose: () => void }) {
  const [initial, setInitial] = useState<{ apiKey: string; logbookId: string } | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [logbookId, setLogbookId] = useState("");
  const [logbooks, setLogbooks] = useState<StationProfile[] | null>(null);
  const [fetchingLogbooks, setFetchingLogbooks] = useState(false);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [showLogbookPicker, setShowLogbookPicker] = useState(false);
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    Promise.all([api.getConfig("wrlApiKey"), api.getConfig("wrlLogbookId")]).then(([k, l]) => {
      setInitial({ apiKey: k ?? "", logbookId: l ?? "" });
      setApiKey(k ?? "");
      setLogbookId(l ?? "");
      if ((k ?? "").trim()) {
        setFetchingLogbooks(true);
        api.wrlFetchLogbooks(k ?? "").then((books) => {
          setLogbooks(books);
          if (books && books.length === 1 && !(l ?? "").trim()) setLogbookId(books[0].station_id);
          setFetchingLogbooks(false);
        });
      }
    });
    api.wrlPendingUploadCount().then(setPendingCount);
  }, []);

  function dismiss() {
    if (initial) {
      api.setConfig("wrlApiKey", initial.apiKey);
      api.setConfig("wrlLogbookId", initial.logbookId);
    }
    onClose();
  }

  async function test() {
    setBusy(true);
    setTestResult(null);
    const key = apiKey.trim();
    if (key) {
      setFetchingLogbooks(true);
      const books = await api.wrlFetchLogbooks(key);
      setLogbooks(books);
      let selected = logbookId;
      if (books && books.length === 1 && !selected.trim()) {
        selected = books[0].station_id;
        setLogbookId(selected);
      }
      setFetchingLogbooks(false);
      const check = await api.wrlTestConnection(key, selected);
      setTestResult({ text: check.ok ? "Pass" : check.detail || "Fail", ok: check.ok });
    } else {
      setTestResult({ text: "Enter your World Radio League API key first.", ok: false });
    }
    setBusy(false);
  }

  async function createLogbook() {
    setCreating(true);
    setCreateError(null);
    try {
      const created = await api.wrlCreateLogbook(apiKey.trim(), "FT8AF", callsign || "");
      setLogbooks([created]);
      setLogbookId(created.station_id);
      const check = await api.wrlTestConnection(apiKey.trim(), created.station_id);
      setTestResult({ text: check.ok ? "Pass" : check.detail || "Fail", ok: check.ok });
    } catch (e) {
      setCreateError(String(e));
    }
    setCreating(false);
  }

  async function save() {
    await Promise.all([api.setConfig("wrlApiKey", apiKey.trim()), api.setConfig("wrlLogbookId", logbookId.trim())]);
    onClose();
  }

  async function upload() {
    setBusy(true);
    setUploadStatus(null);
    const result = await api.wrlUploadPending();
    setBusy(false);
    api.wrlPendingUploadCount().then(setPendingCount);
    if (result.error) {
      setUploadStatus({
        text: `${result.uploaded} uploaded, ${result.failed} failed -- ${result.error}`,
        ok: result.uploaded > 0,
      });
    } else {
      setUploadStatus({ text: `${result.uploaded} uploaded`, ok: true });
    }
  }

  const noLogbooksYet = apiKey.trim() !== "" && logbooks !== null && logbooks.length === 0;

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>World Radio League</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Developer API key from World Radio League -&gt; Integrations -&gt; Developer API.
        </div>
        <div className="field">
          <label>API key</label>
          <input
            value={apiKey}
            onChange={(e) => {
              setApiKey(e.target.value);
              setTestResult(null);
              setLogbooks(null);
              setCreateError(null);
            }}
            placeholder="Your World Radio League Developer API key"
            autoComplete="off"
          />
        </div>
        {fetchingLogbooks ? (
          <div className="muted" style={{ fontSize: 13 }}>Loading logbooks…</div>
        ) : (
          <div className="field">
            <label>Logbook</label>
            <button
              disabled={!logbooks || logbooks.length === 0}
              onClick={() => setShowLogbookPicker(true)}
            >
              {logbookLabel(logbooks ?? [], logbookId)}
            </button>
          </div>
        )}
        {noLogbooksYet && (
          <div className="col" style={{ gap: 6 }}>
            <div className="muted" style={{ fontSize: 12 }}>
              Your World Radio League account has no logbook yet -- WRL won't accept a
              contact until it has one.
            </div>
            <div className="row" style={{ gap: 8, alignItems: "center" }}>
              <button disabled={creating} onClick={createLogbook}>
                {creating ? "Creating…" : "Create \"FT8AF\" logbook"}
              </button>
            </div>
            {createError && <div className="muted" style={{ fontSize: 12, color: "var(--tx)" }}>{createError}</div>}
          </div>
        )}
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
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded.`}
          </div>
          <button disabled={busy || !pendingCount} onClick={upload}>
            {busy ? "Uploading…" : "Upload now"}
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
      {showLogbookPicker && logbooks && logbooks.length > 0 && (
        <ListPickerDialog
          title="Logbook"
          options={logbooks.map((b) => ({ value: b.station_id, label: logbookEntryLabel(b) }))}
          value={logbookId}
          onSelect={(v) => {
            setLogbookId(v);
            setTestResult(null);
            setShowLogbookPicker(false);
          }}
          onCancel={() => setShowLogbookPicker(false)}
        />
      )}
    </div>
  );
}

function logbookEntryLabel(p: StationProfile): string {
  const base = p.profile_name || p.station_id;
  return p.callsign ? `${base} (${p.callsign})` : base;
}

function logbookLabel(books: StationProfile[], selectedId: string): string {
  if (!selectedId.trim()) return "Account default";
  const found = books.find((b) => b.station_id === selectedId);
  return found ? logbookEntryLabel(found) : selectedId;
}

/**
 * HamQTH settings -- password only (HamQTH's own login identifier *is*
 * the operator's callsign, already known from Settings -> Station, so
 * it's shown read-only here rather than asked for again). Test Connection
 * logs in fresh via HamQTH's v2 API; see `hamqth.rs`'s top doc comment.
 */
function HamqthDialog({ callsign, onClose }: { callsign: string; onClose: () => void }) {
  const [initialPassword, setInitialPassword] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    api.getConfig("hamqthPassword").then((v) => {
      setInitialPassword(v ?? "");
      setPassword(v ?? "");
    });
    api.hamqthPendingUploadCount().then(setPendingCount);
  }, []);

  function dismiss() {
    if (initialPassword !== null) api.setConfig("hamqthPassword", initialPassword);
    onClose();
  }

  async function test() {
    await api.setConfig("hamqthPassword", password);
    setBusy(true);
    setTestResult(null);
    const check = await api.hamqthTestConnection(callsign, password);
    setTestResult({ text: check.ok ? "Pass" : check.detail || "Fail", ok: check.ok });
    setBusy(false);
  }

  async function save() {
    await api.setConfig("hamqthPassword", password);
    onClose();
  }

  async function upload() {
    setBusy(true);
    setUploadStatus(null);
    const result = await api.hamqthUploadPending();
    setBusy(false);
    api.hamqthPendingUploadCount().then(setPendingCount);
    if (result.error) {
      setUploadStatus({
        text: `${result.uploaded} uploaded, ${result.already_present} already there, ${result.failed} failed -- ${result.error}`,
        ok: result.uploaded + result.already_present > 0,
      });
    } else {
      setUploadStatus({ text: `${result.uploaded} uploaded, ${result.already_present} already there`, ok: true });
    }
  }

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>HamQTH</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Your hamqth.com website login ({callsign || "your callsign"}) + password.
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

        <div style={{ marginTop: 16, paddingTop: 12, borderTop: "1px solid var(--border)" }}>
          <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded.`}
          </div>
          <button disabled={busy || !pendingCount} onClick={upload}>
            {busy ? "Uploading…" : "Upload now"}
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
 * Club Log settings -- a single per-user API token, created at
 * clublog.org -> Edit Your Settings -> API tab. Not to be confused with
 * `CloudlogDialog` above (a different, unrelated service) -- see
 * `clublog.rs`'s top doc comment.
 */
function ClublogDialog({ onClose }: { onClose: () => void }) {
  const [initialToken, setInitialToken] = useState<string | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    api.getConfig("clublogToken").then((v) => {
      setInitialToken(v ?? "");
      setToken(v ?? "");
    });
    api.clublogPendingUploadCount().then(setPendingCount);
  }, []);

  function dismiss() {
    if (initialToken !== null) api.setConfig("clublogToken", initialToken);
    onClose();
  }

  async function test() {
    await api.setConfig("clublogToken", token.trim());
    setBusy(true);
    setTestResult(null);
    const check = await api.clublogTestConnection(token.trim());
    setTestResult({ text: check.ok ? "Pass" : check.detail || "Fail", ok: check.ok });
    setBusy(false);
  }

  async function save() {
    await api.setConfig("clublogToken", token.trim());
    onClose();
  }

  async function upload() {
    setBusy(true);
    setUploadStatus(null);
    const result = await api.clublogUploadPending();
    setBusy(false);
    api.clublogPendingUploadCount().then(setPendingCount);
    if (result.error) {
      setUploadStatus({
        text: `${result.uploaded} uploaded, ${result.already_present} already there, ${result.failed} failed -- ${result.error}`,
        ok: result.uploaded + result.already_present > 0,
      });
    } else {
      setUploadStatus({ text: `${result.uploaded} uploaded, ${result.already_present} already there`, ok: true });
    }
  }

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>Club Log</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Create a token at clublog.org -&gt; Edit Your Settings -&gt; API. Not the same as
          Cloudlog above -- Club Log is a different service.
        </div>
        <div className="field">
          <label>API token</label>
          <input
            value={token}
            onChange={(e) => {
              setToken(e.target.value);
              setTestResult(null);
            }}
            placeholder="cl_..."
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
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded.`}
          </div>
          <button disabled={busy || !pendingCount} onClick={upload}>
            {busy ? "Uploading…" : "Upload now"}
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
 * HRDLog.net settings -- callsign (read-only, same as Settings -> Station)
 * + upload Code. **Lower confidence than the other dialogs here** -- see
 * `hrdlog.rs`'s top doc comment: the real API docs were unreachable during
 * development, so Test Connection shows HRDLog's raw response text rather
 * than a plain Pass/Fail, since this app can't yet reliably tell which is
 * which.
 */
function HrdlogDialog({ callsign, onClose }: { callsign: string; onClose: () => void }) {
  const [initialCode, setInitialCode] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    api.getConfig("hrdlogCode").then((v) => {
      setInitialCode(v ?? "");
      setCode(v ?? "");
    });
    api.hrdlogPendingUploadCount().then(setPendingCount);
  }, []);

  function dismiss() {
    if (initialCode !== null) api.setConfig("hrdlogCode", initialCode);
    onClose();
  }

  async function test() {
    await api.setConfig("hrdlogCode", code.trim());
    setBusy(true);
    setTestResult(null);
    const check = await api.hrdlogTestConnection(callsign, code.trim());
    setTestResult({ text: check.detail || (check.ok ? "No response body" : "Fail"), ok: check.ok });
    setBusy(false);
  }

  async function save() {
    await api.setConfig("hrdlogCode", code.trim());
    onClose();
  }

  async function upload() {
    setBusy(true);
    setUploadStatus(null);
    const result = await api.hrdlogUploadPending();
    setBusy(false);
    api.hrdlogPendingUploadCount().then(setPendingCount);
    if (result.error) {
      setUploadStatus({ text: `${result.uploaded} uploaded, ${result.failed} failed -- ${result.error}`, ok: result.uploaded > 0 });
    } else {
      setUploadStatus({ text: `${result.uploaded} uploaded`, ok: true });
    }
  }

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>HRDLog.net</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Your hrdlog.net Code ({callsign || "your callsign"}), emailed after registration.
          This integration's API details couldn't be confirmed from HRDLog's own docs during
          development -- test it live before relying on it.
        </div>
        <div className="field">
          <label>Code</label>
          <input
            value={code}
            onChange={(e) => {
              setCode(e.target.value);
              setTestResult(null);
            }}
            autoComplete="off"
          />
        </div>
        <div className="row" style={{ gap: 8, marginTop: 10, alignItems: "center" }}>
          <button disabled={busy} onClick={test}>{busy ? "Working…" : "Test Connection"}</button>
        </div>
        {testResult && (
          <div className="muted" style={{ fontSize: 12, color: testResult.ok ? "var(--ok)" : "var(--tx)" }}>
            {testResult.text}
          </div>
        )}

        <div style={{ marginTop: 16, paddingTop: 12, borderTop: "1px solid var(--border)" }}>
          <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded.`}
          </div>
          <button disabled={busy || !pendingCount} onClick={upload}>
            {busy ? "Uploading…" : "Upload now"}
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
 * eQSL settings -- username/password (upload) + QTH Nickname (download
 * only). Two real, separate operations -- see `eqsl.rs`'s top doc
 * comment for the confidence caveat (lower than every other integration
 * here except HRDLog.net). Download never inserts a new QSO -- only
 * updates a matched existing one.
 */
function EqslDialog({ onClose }: { onClose: () => void }) {
  const [initial, setInitial] = useState<{ username: string; password: string; nickname: string } | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [nickname, setNickname] = useState("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ text: string; ok: boolean } | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [uploadStatus, setUploadStatus] = useState<{ text: string; ok: boolean } | null>(null);
  const [downloadStatus, setDownloadStatus] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    Promise.all([api.getConfig("eqslUsername"), api.getConfig("eqslPassword"), api.getConfig("eqslNickname")]).then(
      ([u, p, n]) => {
        setInitial({ username: u ?? "", password: p ?? "", nickname: n ?? "" });
        setUsername(u ?? "");
        setPassword(p ?? "");
        setNickname(n ?? "");
      }
    );
    api.eqslPendingUploadCount().then(setPendingCount);
  }, []);

  function dismiss() {
    if (initial) {
      api.setConfig("eqslUsername", initial.username);
      api.setConfig("eqslPassword", initial.password);
      api.setConfig("eqslNickname", initial.nickname);
    }
    onClose();
  }

  async function saveFields() {
    await Promise.all([
      api.setConfig("eqslUsername", username.trim()),
      api.setConfig("eqslPassword", password),
      api.setConfig("eqslNickname", nickname.trim()),
    ]);
  }

  async function test() {
    await saveFields();
    setBusy(true);
    setTestResult(null);
    const check = await api.eqslTestConnection(username.trim(), password);
    setTestResult({ text: check.detail || (check.ok ? "No response body" : "Fail"), ok: check.ok });
    setBusy(false);
  }

  async function save() {
    await saveFields();
    onClose();
  }

  async function upload() {
    setBusy(true);
    setUploadStatus(null);
    const result = await api.eqslUploadPending();
    setBusy(false);
    api.eqslPendingUploadCount().then(setPendingCount);
    if (result.error) {
      setUploadStatus({
        text: `${result.uploaded} uploaded, ${result.already_present} already there, ${result.failed} failed -- ${result.error}`,
        ok: result.uploaded + result.already_present > 0,
      });
    } else {
      setUploadStatus({ text: `${result.uploaded} uploaded, ${result.already_present} already there`, ok: true });
    }
  }

  async function download(fullRefresh: boolean) {
    await saveFields();
    setBusy(true);
    setDownloadStatus(null);
    const result = await api.eqslDownloadConfirmations(fullRefresh);
    setBusy(false);
    setDownloadStatus(
      result.error
        ? { text: result.error, ok: false }
        : { text: `${result.newly_confirmed} QSOs newly confirmed (${result.unmatched} not found locally)`, ok: true }
    );
  }

  return (
    <div className="modal-overlay" onClick={dismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>eQSL</h3>
          <button className="modal-close" aria-label="Close" onClick={dismiss}>✕</button>
        </div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          Your eqsl.cc website login. This integration's API details couldn't be confirmed
          from eQSL's own docs during development -- test it live before relying on it.
        </div>
        <div className="field">
          <label>Username</label>
          <input value={username} onChange={(e) => { setUsername(e.target.value); setTestResult(null); }} autoComplete="off" />
        </div>
        <div className="field">
          <label>Password</label>
          <input
            type="password"
            value={password}
            onChange={(e) => { setPassword(e.target.value); setTestResult(null); }}
            autoComplete="off"
          />
        </div>
        <div className="field">
          <label>QTH Nickname (for download)</label>
          <input value={nickname} onChange={(e) => setNickname(e.target.value)} placeholder="Matches your eQSL profile" autoComplete="off" />
        </div>
        <div className="row" style={{ gap: 8, marginTop: 10, alignItems: "center" }}>
          <button disabled={busy} onClick={test}>{busy ? "Working…" : "Test Connection"}</button>
        </div>
        {testResult && (
          <div className="muted" style={{ fontSize: 12, color: testResult.ok ? "var(--ok)" : "var(--tx)" }}>
            {testResult.text}
          </div>
        )}

        <div style={{ marginTop: 16, paddingTop: 12, borderTop: "1px solid var(--border)" }}>
          <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
            {pendingCount == null ? "Checking…" : `${pendingCount} QSOs not yet uploaded.`}
          </div>
          <button disabled={busy || !pendingCount} onClick={upload}>{busy ? "Uploading…" : "Upload now"}</button>
          {uploadStatus && (
            <div className="muted" style={{ marginTop: 8, fontSize: 12, color: uploadStatus.ok ? "var(--ok)" : "var(--tx)" }}>
              {uploadStatus.text}
            </div>
          )}
        </div>

        <div style={{ marginTop: 16, paddingTop: 12, borderTop: "1px solid var(--border)" }}>
          <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
            Download newly confirmed QSLs and merge them into this log (never inserts a new QSO).
          </div>
          <div className="row" style={{ gap: 8 }}>
            <button disabled={busy} onClick={() => download(false)}>{busy ? "Working…" : "Download new"}</button>
            <button disabled={busy} onClick={() => download(true)}>Download all</button>
          </div>
          {downloadStatus && (
            <div className="muted" style={{ marginTop: 8, fontSize: 12, color: downloadStatus.ok ? "var(--ok)" : "var(--tx)" }}>
              {downloadStatus.text}
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
