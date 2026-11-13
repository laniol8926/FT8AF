// Pure, testable logic for Settings -> About's "Report a Bug" row, ported
// from the Android fork's own `BugReportLogic.kt` almost verbatim --
// desktop has no device/OS version block to attach (no Android release/SDK/
// manufacturer concept), so that part of the info block is swapped for a
// platform string instead (see `BugReportInfo`).

/** GitHub `owner/repo` for issue submission -- matches the link in AboutSettings. */
export const BUG_REPORT_REPO = "patrickrb/FT8AF";

export interface BugReportInfo {
  appVersion: string;
  buildDate: string;
  /** e.g. "linux-x86_64", "windows-x86_64" -- `navigator.platform`-derived, not an OS version. */
  platform: string;
  callsign: string;
}

function buildReportBody(heading: string, text: string, blankPlaceholder: string, info: BugReportInfo): string {
  const callsign = info.callsign.trim() || "(not set)";
  return (
    `${heading}\n` +
    `${text.trim() || blankPlaceholder}\n\n` +
    `--- App / device info ---\n` +
    `App version: ${info.appVersion}\n` +
    `Build date: ${info.buildDate}\n` +
    `Platform: ${info.platform}\n` +
    `Callsign: ${callsign}\n`
  );
}

/** Developer-facing report body: the user's description followed by an app/platform block. */
export function buildBugReportBody(description: string, info: BugReportInfo): string {
  return buildReportBody("Describe the problem:", description, "(no description provided)", info);
}

/** Short issue/email title, e.g. "Bug report (v0.1.2)". */
export function buildBugReportTitle(info: BugReportInfo): string {
  return `Bug report (v${info.appVersion})`;
}

/**
 * Prefilled GitHub "new issue" URL. GitHub issues can't carry a file
 * attachment, so the body notes where to find the app's logs if asked (once
 * desktop has a debug-log file to point at -- see `AboutSettings.tsx`'s own
 * doc comment on why Share/View logs are still stubs).
 */
export function buildGithubIssueUrl(description: string, info: BugReportInfo): string {
  const title = buildBugReportTitle(info);
  const body = buildBugReportBody(description, info);
  return (
    `https://github.com/${BUG_REPORT_REPO}/issues/new` +
    `?title=${encodeURIComponent(title)}&body=${encodeURIComponent(body)}`
  );
}
