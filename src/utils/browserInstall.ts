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
