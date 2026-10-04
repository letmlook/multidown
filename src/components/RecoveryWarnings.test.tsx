import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RecoveryWarning } from "../types/download";
import { RecoveryWarnings } from "./RecoveryWarnings";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

function warning(over: Partial<RecoveryWarning> = {}): RecoveryWarning {
  return {
    id: "w-1",
    domain: "example.com",
    message: "记录格式异常，已隔离原文件",
    recovery_path: "/data/quarantine/example.com/record.json",
    record_key: "example.com|https://example.com/a.bin",
    ...over,
  };
}

function mockList(warnings: RecoveryWarning[]): void {
  invoke.mockImplementation((command: string) => {
    if (command === "list_recovery_warnings") return Promise.resolve(warnings);
    return Promise.resolve(undefined);
  });
}

describe("RecoveryWarnings", () => {
  beforeEach(() => {
    invoke.mockReset();
    mockList([]);
  });

  it("fetches_the_warning_list_once_on_mount", async () => {
    mockList([warning()]);
    render(<RecoveryWarnings />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledTimes(1);
    });
    expect(invoke).toHaveBeenCalledWith("list_recovery_warnings");
  });

  it("lists_each_warning_domain_and_message", async () => {
    mockList([
      warning(),
      warning({ id: "w-2", domain: "cdn.example.org", message: "缺少校验头，无法续传" }),
    ]);
    render(<RecoveryWarnings />);

    expect(await screen.findByText("example.com")).toBeInTheDocument();
    expect(screen.getByText("记录格式异常，已隔离原文件")).toBeInTheDocument();
    expect(screen.getByText("cdn.example.org")).toBeInTheDocument();
    expect(screen.getByText("缺少校验头，无法续传")).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("shows_the_recovery_path_as_evidence_for_each_warning", async () => {
    mockList([warning()]);
    render(<RecoveryWarnings />);

    expect(await screen.findByText(/data\/quarantine\/example\.com\/record\.json/)).toBeInTheDocument();
  });

  it("omits_the_recovery_path_row_when_the_backend_has_none", async () => {
    mockList([warning({ id: "w-2", recovery_path: null, message: "记录缺少保存路径" })]);
    render(<RecoveryWarnings />);

    await screen.findByText("记录缺少保存路径");
    expect(screen.queryByText(/隔离或备份位置/)).not.toBeInTheDocument();
  });

  it("acknowledges_only_the_dismissed_warning_id", async () => {
    mockList([warning(), warning({ id: "w-2", domain: "cdn.example.org" })]);
    render(<RecoveryWarnings />);
    await screen.findByText("example.com");

    fireEvent.click(screen.getByRole("button", { name: "忽略警告：example.com" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("acknowledge_recovery_warnings", { ids: ["w-1"] });
    });
    expect(screen.queryByText("example.com")).not.toBeInTheDocument();
    // 其余警告仍在列表中
    expect(screen.getByText("cdn.example.org")).toBeInTheDocument();
  });

  it("acknowledges_every_listed_id_from_the_dismiss_all_button", async () => {
    mockList([warning(), warning({ id: "w-2", domain: "cdn.example.org" })]);
    render(<RecoveryWarnings />);
    await screen.findByText("example.com");

    fireEvent.click(screen.getByRole("button", { name: "全部忽略" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("acknowledge_recovery_warnings", {
        ids: ["w-1", "w-2"],
      });
    });
    expect(screen.queryByText("cdn.example.org")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByRole("region", { name: "启动恢复警告" })).not.toBeInTheDocument();
    });
  });

  it("keeps_the_warning_visible_when_acknowledgement_fails", async () => {
    mockList([warning()]);
    invoke.mockImplementation((command: string) => {
      if (command === "list_recovery_warnings") return Promise.resolve([warning()]);
      return Promise.reject("确认失败");
    });
    render(<RecoveryWarnings />);
    await screen.findByText("example.com");

    fireEvent.click(screen.getByRole("button", { name: "忽略警告：example.com" }));

    expect(await screen.findByText(/确认失败/)).toBeInTheDocument();
    expect(screen.getByText("example.com")).toBeInTheDocument();
  });

  it("renders_nothing_when_there_are_no_warnings", async () => {
    mockList([]);
    render(<RecoveryWarnings />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("list_recovery_warnings");
    });
    expect(screen.queryByRole("region", { name: "启动恢复警告" })).not.toBeInTheDocument();
  });

  it("shows_a_retry_affordance_when_the_list_cannot_be_loaded", async () => {
    invoke.mockImplementation((command: string) =>
      command === "list_recovery_warnings" ? Promise.reject("后端不可用") : Promise.resolve(undefined)
    );
    render(<RecoveryWarnings />);

    expect(await screen.findByText(/后端不可用/)).toBeInTheDocument();

    mockList([warning()]);
    fireEvent.click(screen.getByRole("button", { name: "重试" }));

    expect(await screen.findByText("example.com")).toBeInTheDocument();
  });
});
