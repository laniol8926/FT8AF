import { describe, it, expect } from "vitest";
import { buildBugReportBody, buildBugReportTitle, buildGithubIssueUrl, BUG_REPORT_REPO } from "./bugReport";

const INFO = { appVersion: "0.1.2", buildDate: "2026-11-13", platform: "linux-x86_64", callsign: "AI5II" };

describe("buildBugReportTitle", () => {
  it("includes the app version", () => {
    expect(buildBugReportTitle(INFO)).toBe("Bug report (v0.1.2)");
  });
});

describe("buildBugReportBody", () => {
  it("includes the description and the app/device block", () => {
    const body = buildBugReportBody("Tune button does nothing", INFO);
    expect(body).toContain("Describe the problem:");
    expect(body).toContain("Tune button does nothing");
    expect(body).toContain("App version: 0.1.2");
    expect(body).toContain("Build date: 2026-11-13");
    expect(body).toContain("Platform: linux-x86_64");
    expect(body).toContain("Callsign: AI5II");
  });

  it("falls back to a placeholder for a blank description", () => {
    expect(buildBugReportBody("   ", INFO)).toContain("(no description provided)");
  });

  it("falls back to a placeholder for a blank callsign", () => {
    expect(buildBugReportBody("x", { ...INFO, callsign: "" })).toContain("Callsign: (not set)");
  });
});

describe("buildGithubIssueUrl", () => {
  it("targets the real repo with an encoded title and body", () => {
    const url = buildGithubIssueUrl("Tune does nothing", INFO);
    expect(url.startsWith(`https://github.com/${BUG_REPORT_REPO}/issues/new?`)) .toBe(true);
    expect(url).toContain("title=Bug%20report");
    expect(url).toContain("Tune%20does%20nothing");
  });
});
