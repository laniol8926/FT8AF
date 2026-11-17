import { describe, expect, it } from "vitest";
import {
  buildQsoSteps,
  computeBeamHeadings,
  computeCurrentStepIndex,
  computeDistanceText,
  computeGrayLineStatusText,
  computeLastHeard,
  formatLocationLine,
  grayLineStatusText,
  lastHeardText,
  qsoLiveStatusText,
} from "./qsoSheet";

describe("computeDistanceText", () => {
  it("is the placeholder when either grid is missing", () => {
    expect(computeDistanceText(null, "FN42", true)).toBe("--");
    expect(computeDistanceText("FN42", null, true)).toBe("--");
    expect(computeDistanceText("FN42", "", true)).toBe("--");
    expect(computeDistanceText("", "", true)).toBe("--");
  });

  it("is the placeholder when a grid cannot be parsed", () => {
    expect(computeDistanceText("FN42", "AB", true)).toBe("--");
  });

  it("is a real zero (not the placeholder) for identical grids", () => {
    expect(computeDistanceText("FN42", "FN42", true)).toBe("0 mi");
    expect(computeDistanceText("FN42", "FN42", false)).toBe("0 km");
  });

  it("is a number with the chosen unit for distinct grids", () => {
    const miles = computeDistanceText("FN42", "IO91", true);
    expect(miles).not.toBe("--");
    expect(miles).toMatch(/^\d+ mi$/);
    const km = computeDistanceText("FN42", "IO91", false);
    expect(km).toMatch(/^\d+ km$/);
  });
});

describe("computeBeamHeadings", () => {
  it("is null when either grid is missing or unparseable", () => {
    expect(computeBeamHeadings(null, "FN42")).toBeNull();
    expect(computeBeamHeadings("FN42", "AB")).toBeNull();
  });

  it("is null for identical points (a bearing to yourself is undefined)", () => {
    expect(computeBeamHeadings("FN42", "FN42")).toBeNull();
  });

  it("the long path is the short path's reciprocal", () => {
    const h = computeBeamHeadings("FN42", "IO91")!;
    expect(h).not.toBeNull();
    expect(h.longPathDeg).toBe((h.shortPathDeg + 180) % 360);
  });
});

describe("computeLastHeard / lastHeardText", () => {
  it("is unknown for a missing/non-positive timestamp", () => {
    expect(lastHeardText(computeLastHeard(0, 1000))).toBe("--");
    expect(lastHeardText(computeLastHeard(-5, 1000))).toBe("--");
  });

  it("clamps a future timestamp to just now, never negative", () => {
    expect(lastHeardText(computeLastHeard(5000, 1000))).toBe("just now");
  });

  it("buckets seconds/minutes/hours/days", () => {
    const now = 1_700_000_000_000; // a real epoch instant, large enough that now - 5 days stays positive
    expect(lastHeardText(computeLastHeard(now - 10_000, now))).toBe("10s ago");
    expect(lastHeardText(computeLastHeard(now - 3 * 60_000, now))).toBe("3m ago");
    expect(lastHeardText(computeLastHeard(now - 2 * 3600_000, now))).toBe("2h ago");
    expect(lastHeardText(computeLastHeard(now - 5 * 86_400_000, now))).toBe("5d ago");
  });
});

describe("formatLocationLine", () => {
  it("is null when nothing is usable", () => {
    expect(formatLocationLine(null, null, null)).toBeNull();
  });

  it("combines state + country", () => {
    expect(formatLocationLine("TX", "United States", "EM71")).toBe("TX, United States • EM71");
  });

  it("falls back to country alone when state is absent", () => {
    expect(formatLocationLine(null, "Japan", null)).toBe("Japan");
  });

  it("falls back to grid alone when state/country are absent", () => {
    expect(formatLocationLine(null, null, "PM95")).toBe("PM95");
  });
});

describe("computeCurrentStepIndex", () => {
  it("is -1 when just viewing (no live QSO, not fully complete)", () => {
    expect(computeCurrentStepIndex(false, false, null)).toBe(-1);
  });

  it("is 5 when fully worked and confirmed, not live", () => {
    expect(computeCurrentStepIndex(true, false, null)).toBe(5);
  });

  it("maps each live stage to its step index", () => {
    expect(computeCurrentStepIndex(false, true, "grid")).toBe(0);
    expect(computeCurrentStepIndex(false, true, "report")).toBe(1);
    expect(computeCurrentStepIndex(false, true, "r_report")).toBe(2);
    expect(computeCurrentStepIndex(false, true, "rr73")).toBe(3);
    expect(computeCurrentStepIndex(false, true, "bye73")).toBe(4);
  });

  it("is -1 for an unmapped live stage", () => {
    expect(computeCurrentStepIndex(false, true, "cq")).toBe(-1);
    expect(computeCurrentStepIndex(false, true, "idle")).toBe(-1);
  });
});

describe("buildQsoSteps", () => {
  it("builds the 5 real exchange previews", () => {
    const steps = buildQsoSteps("K1ABC", "AI5II", "EM71", "-10");
    expect(steps).toHaveLength(5);
    expect(steps[0].messagePreview).toBe("K1ABC AI5II EM71");
    expect(steps[1].messagePreview).toBe("AI5II K1ABC -10");
    expect(steps[2].messagePreview).toBe("K1ABC AI5II R-10");
    expect(steps[3].messagePreview).toBe("AI5II K1ABC RR73");
    expect(steps[4].messagePreview).toBe("K1ABC AI5II 73");
  });
});

describe("qsoLiveStatusText", () => {
  it("shows sending + the step label while transmitting live", () => {
    expect(qsoLiveStatusText("K1ABC", true, true, 1)).toBe("QSOing with K1ABC — Sending report sent");
  });

  it("shows waiting while live but not transmitting", () => {
    expect(qsoLiveStatusText("K1ABC", true, false, 1)).toBe("Waiting for K1ABC to reply...");
  });

  it("shows a generic transmitting line when not live but TX is on", () => {
    expect(qsoLiveStatusText("K1ABC", false, true, -1)).toBe("Transmitting...");
  });

  it("is null when neither applies", () => {
    expect(qsoLiveStatusText("K1ABC", false, false, -1)).toBeNull();
  });
});

describe("grayLineStatusText / computeGrayLineStatusText", () => {
  it("formats daylight + sunset countdown", () => {
    const text = grayLineStatusText({ phase: "daylight", detail: "sunset", countdown: "1h 20m" });
    expect(text).toBe("Daylight · sunset in 1h 20m");
  });

  it("formats the imminent-event now-string", () => {
    const text = grayLineStatusText({ phase: "gray_line", detail: "sunset", countdown: "" });
    expect(text).toBe("On the gray line now · at sunset");
  });

  it("is null for an unknown grid", () => {
    expect(computeGrayLineStatusText(null, Date.now())).toBeNull();
    expect(computeGrayLineStatusText("", Date.now())).toBeNull();
  });

  it("is a real string for a known grid", () => {
    const text = computeGrayLineStatusText("FN42", Date.now());
    expect(text).not.toBeNull();
    expect(text!.length).toBeGreaterThan(0);
  });
});
