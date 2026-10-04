import { describe, expect, it } from "vitest";
import {
  formatBrowserInstallOutcome,
  formatExtensionDirectoryHint,
  formatExtensionZipHint,
} from "./browserInstall";

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

describe("formatExtensionDirectoryHint", () => {
  it("names_the_directory_and_the_unpacked_load_flow", () => {
    const hint = formatExtensionDirectoryHint("C:\\Users\\me\\AppData\\extension");

    expect(hint).toContain("已解压的扩展目录：C:\\Users\\me\\AppData\\extension");
    expect(hint).toContain("chrome://extensions");
    expect(hint).toContain("加载已解压的扩展程序");
  });
});

describe("formatExtensionZipHint", () => {
  it("names_the_zip_and_states_it_must_be_extracted_first", () => {
    const hint = formatExtensionZipHint("C:\\Users\\me\\AppData\\extension\\multidown-extension.zip");

    expect(hint).toContain("已导出扩展压缩包：C:\\Users\\me\\AppData\\extension\\multidown-extension.zip");
    expect(hint).toContain("请先解压");
    expect(hint).toContain("不能直接加载 ZIP");
  });
});
