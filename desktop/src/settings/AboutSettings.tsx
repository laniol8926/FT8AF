import { useEffect, useState } from "react";
import { api } from "../ipc";
import { buildBugReportBody, buildBugReportTitle, buildGithubIssueUrl } from "../bugReport";
import { SettingsRow } from "./SettingsPrimitives";

const WEBSITE_URL = "https://ft8af.app";
const GITHUB_URL = "https://github.com/patrickrb/FT8AF";
const DISCORD_URL = "https://discord.gg/UeE3ZpwRG";
const AUTHOR_K1AF_URL = "https://www.qrz.com/db/K1AF";
const AUTHOR_N0RC_URL = "https://www.qrz.com/db/N0RC";
const INTER_FONT_URL = "https://github.com/rsms/inter";
const BUG_REPORT_EMAIL = "k1af@FT8AF.app";

/**
 * About / support, ported from the Android fork's own `AboutSettings.kt`:
 * version/build-date row, an About/FAQ dialog (website/community/author
 * links, same exact copy and URLs), and "Report a Bug" (the pure logic in
 * `bugReport.ts`, ported from `BugReportLogic.kt` -- email or GitHub issue,
 * both prefilled). "Share logs"/"View logs" are the one deliberate gap:
 * Android's version reads/shares a `debug.log` file its own `fileLog()`
 * writes throughout the app (per this repo's own AGENTS.md) -- desktop has
 * no such structured log file at all yet (confirmed: no file-logging sink
 * anywhere in `src-tauri/src/`), so these stay disabled stubs with Android's
 * real copy rather than faking a working log viewer. Real future work, not
 * a UI-only port.
 */
export function AboutSettings({ call }: { call: string }) {
  const [version, setVersion] = useState("");
  const [buildDate, setBuildDate] = useState("");
  const [showAbout, setShowAbout] = useState(false);
  const [showBugReport, setShowBugReport] = useState(false);

  useEffect(() => {
    api.getAppInfo().then((info) => {
      setVersion(info.version);
      setBuildDate(info.build_date);
    });
  }, []);

  return (
    <>
      <div className="panel">
        <h3>About</h3>
        <div className="settings-category-list">
          <SettingsRow label="FT8AF" description={`Build ${buildDate}`} value={`v${version}`} />
          <SettingsRow label="FAQ & Support" onClick={() => setShowAbout(true)} />
          <SettingsRow label="Report a Bug" onClick={() => setShowBugReport(true)} />
          <SettingsRow label="Share logs" description="Send debug.log to email, Discord, Drive…" disabled />
          <SettingsRow label="View logs" description="View / share debug.log" disabled />
        </div>
      </div>

      {showAbout && <AboutDialog onDismiss={() => setShowAbout(false)} version={version} buildDate={buildDate} />}
      {showBugReport && (
        <BugReportDialog onDismiss={() => setShowBugReport(false)} version={version} buildDate={buildDate} call={call} />
      )}
    </>
  );
}

function LinkLine({ href, children }: { href: string; children: string }) {
  return (
    <div
      className="about-link"
      role="button"
      onClick={() => api.openExternal(href)}
    >
      {children}
    </div>
  );
}

function AboutDialog({
  onDismiss,
  version,
  buildDate,
}: {
  onDismiss: () => void;
  version: string;
  buildDate: string;
}) {
  return (
    <div className="modal-overlay" onClick={onDismiss}>
      <div className="modal about-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>FT8AF</h3>
        <p className="muted" style={{ lineHeight: 1.5 }}>
          Version {version} (build {buildDate})
          <br />
          FT8, made easy.
          <br />
          <br />
          A standalone FT8 transceiver app. Hamlib/FLrig/direct CI-V rig control
          with automatic sequencing and logging.
        </p>
        <div className="col" style={{ gap: 10 }}>
          <div>
            <div style={{ fontWeight: 600 }}>Website</div>
            <LinkLine href={WEBSITE_URL}>ft8af.app</LinkLine>
          </div>
          <div>
            <div style={{ fontWeight: 600 }}>Community</div>
            <LinkLine href={GITHUB_URL}>github.com/patrickrb/FT8AF</LinkLine>
            <LinkLine href={DISCORD_URL}>discord.gg/UeE3ZpwRG</LinkLine>
          </div>
          <div>
            <div style={{ fontWeight: 600 }}>Built by</div>
            <LinkLine href={AUTHOR_K1AF_URL}>K1AF — Patrick Burns (QRZ)</LinkLine>
            <LinkLine href={AUTHOR_N0RC_URL}>N0RC — Reid (QRZ)</LinkLine>
          </div>
          <div className="muted" style={{ fontSize: 12 }}>
            <LinkLine href={INTER_FONT_URL}>UI font: Inter by Rasmus Andersson (SIL OFL 1.1)</LinkLine>
          </div>
        </div>
        <div className="actions">
          <button className="modal-text-btn modal-text-btn-accent" onClick={onDismiss}>OK</button>
        </div>
      </div>
    </div>
  );
}

function BugReportDialog({
  onDismiss,
  version,
  buildDate,
  call,
}: {
  onDismiss: () => void;
  version: string;
  buildDate: string;
  call: string;
}) {
  const [description, setDescription] = useState("");
  const info = { appVersion: version, buildDate, platform: navigator.platform || "unknown", callsign: call };

  function sendEmail() {
    const subject = buildBugReportTitle(info);
    const body = buildBugReportBody(description, info);
    api.openExternal(`mailto:${BUG_REPORT_EMAIL}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`);
    onDismiss();
  }

  function openGithub() {
    api.openExternal(buildGithubIssueUrl(description, info));
    onDismiss();
  }

  return (
    <div className="modal-overlay" onClick={onDismiss}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Report a Bug</h3>
        <div className="col">
          <div className="field">
            <label>What went wrong?</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Describe what went wrong…"
              rows={5}
              autoFocus
            />
          </div>
          <span className="muted" style={{ fontSize: 12 }}>
            Your app version, build date, platform, and callsign are attached automatically.
          </span>
        </div>
        <div className="actions">
          <button className="modal-text-btn" onClick={onDismiss}>Cancel</button>
          <button className="modal-text-btn" onClick={openGithub}>Open GitHub issue</button>
          <button className="modal-text-btn modal-text-btn-accent" onClick={sendEmail}>Send email</button>
        </div>
      </div>
    </div>
  );
}
