import type { BrowserInstallOutcome } from "../types/download";

export function formatBrowserInstallOutcome(outcome: BrowserInstallOutcome): string {
  const sections: string[] = [];
  if (outcome.opened.length > 0) {
    sections.push(`已打开：${outcome.opened.join("、")}`);
  }
  if (outcome.manual_steps.length > 0) {
    sections.push(outcome.manual_steps.join("\n"));
  }
  return sections.join("\n\n");
}

/** 已解压扩展目录的说明：Chromium 只能加载解压后的目录。 */
export function formatExtensionDirectoryHint(directory: string): string {
  return [
    `已解压的扩展目录：${directory}`,
    "在 chrome://extensions 打开「开发者模式」，点击「加载已解压的扩展程序」，选择该目录即可。",
  ].join("\n");
}

/** 导出 ZIP 的说明：ZIP 需先解压，浏览器不能直接加载。 */
export function formatExtensionZipHint(zipPath: string): string {
  return [
    `已导出扩展压缩包：${zipPath}`,
    "Chrome / Edge 不能直接加载 ZIP，请先解压，再在 chrome://extensions 用「加载已解压的扩展程序」选择解压后的目录。",
  ].join("\n");
}
