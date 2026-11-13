import { describe, it, expect } from "vitest";
import { defaultBackupFileName } from "./settingsBackup";

describe("defaultBackupFileName", () => {
  it("builds the expected name", () => {
    expect(defaultBackupFileName("2026-11-14", "0.1.2")).toBe("ft8af-settings-2026-11-14-v0.1.2.json");
  });

  it("sanitizes unsafe characters in the version", () => {
    expect(defaultBackupFileName("2026-11-14", "0.1.2 dev/beta")).toBe(
      "ft8af-settings-2026-11-14-v0.1.2_dev_beta.json",
    );
  });

  it("falls back to 'unknown' for a blank version", () => {
    expect(defaultBackupFileName("2026-11-14", "")).toBe("ft8af-settings-2026-11-14-vunknown.json");
    expect(defaultBackupFileName("2026-11-14", "   ")).toBe("ft8af-settings-2026-11-14-vunknown.json");
  });
});
