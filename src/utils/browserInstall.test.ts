import { describe, expect, it } from "vitest";
import { formatBrowserInstallOutcome } from "./browserInstall";

describe("formatBrowserInstallOutcome", () => {
  it("formats_opened_browsers_and_manual_steps", () => {
    expect(
      formatBrowserInstallOutcome({
        opened: ["Google Chrome"],
        manual_steps: ["请确认开发者模式已启用。"],
      }),
    ).toBe("已打开：Google Chrome\n\n请确认开发者模式已启用。");
  });

  it("formats_manual_only_outcome", () => {
    expect(
      formatBrowserInstallOutcome({
        opened: [],
        manual_steps: ["请手动打开扩展管理页。"],
      }),
    ).toBe("请手动打开扩展管理页。");
  });
});
