import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TaskInfo } from "../types/download";
import { DeleteTaskModal } from "./DeleteTaskModal";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const task: TaskInfo = {
  id: "task-1",
  url: "https://example.com/a.bin",
  filename: "a.bin",
  save_path: "/downloads/a.bin",
  total_bytes: 100,
  downloaded_bytes: 40,
  status: "paused",
  error_message: null,
  speed_bps: null,
  created_at: 1,
  kind: "http",
  upload_speed_bps: null,
  uploaded_bytes: null,
  peers: null,
  seeds: null,
  files: null,
  metadata_ready: true,
};

function preview(overrides: Partial<{ paths: string[]; can_delete_files: boolean }> = {}) {
  return {
    task_id: "task-1",
    paths: ["/downloads/a.bin"],
    can_delete_files: true,
    ...overrides,
  };
}

describe("DeleteTaskModal", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((command: string) => {
      if (command === "preview_task_deletion") return Promise.resolve(preview());
      return Promise.resolve(undefined);
    });
  });

  it("shows_preview_paths_and_defaults_the_file_checkbox_off", async () => {
    render(<DeleteTaskModal tasks={[task]} onClose={vi.fn()} onConfirmed={vi.fn()} />);

    expect(await screen.findByText("/downloads/a.bin")).toBeInTheDocument();
    const checkbox = screen.getByRole("checkbox", { name: /同时删除文件/ });
    expect(checkbox).not.toBeChecked();
  });

  it("disables_file_deletion_when_preview_reports_targets_outside_the_save_root", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "preview_task_deletion") {
        return Promise.resolve(preview({ can_delete_files: false, paths: [] }));
      }
      return Promise.resolve(undefined);
    });
    render(<DeleteTaskModal tasks={[task]} onClose={vi.fn()} onConfirmed={vi.fn()} />);

    expect(await screen.findByRole("checkbox", { name: /同时删除文件/ })).toBeDisabled();
  });

  it("cancel_closes_without_invoking_any_removal", async () => {
    const onClose = vi.fn();
    render(<DeleteTaskModal tasks={[task]} onClose={onClose} onConfirmed={vi.fn()} />);
    await screen.findByText("/downloads/a.bin");

    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    expect(onClose).toHaveBeenCalledOnce();
    await waitFor(() => {
      expect(invoke).not.toHaveBeenCalledWith("remove_task", expect.anything());
    });
  });

  it("confirm_sends_the_selected_delete_files_value_for_every_task", async () => {
    const onConfirmed = vi.fn();
    const second = { ...task, id: "task-2", filename: "b.bin" };
    invoke.mockImplementation((_command: string, args?: { taskId?: string }) => {
      if (args?.taskId === "task-1") {
        return Promise.resolve(preview({ paths: ["/downloads/a.bin"] }));
      }
      if (args?.taskId === "task-2") {
        return Promise.resolve(preview({ paths: ["/downloads/b.bin"] }));
      }
      return Promise.resolve(undefined);
    });
    render(
      <DeleteTaskModal tasks={[task, second]} onClose={vi.fn()} onConfirmed={onConfirmed} />,
    );
    await screen.findByText("/downloads/b.bin");

    // 默认不勾选：仅移除记录，保留文件
    fireEvent.click(screen.getByRole("button", { name: "确认移除" }));
    await waitFor(() => {
      expect(onConfirmed).toHaveBeenCalledOnce();
    });
    expect(invoke).toHaveBeenCalledWith("remove_task", {
      taskId: "task-1",
      deleteFiles: false,
    });
    expect(invoke).toHaveBeenCalledWith("remove_task", {
      taskId: "task-2",
      deleteFiles: false,
    });
  });

  it("confirm_after_checking_the_box_requests_file_deletion", async () => {
    const onConfirmed = vi.fn();
    render(<DeleteTaskModal tasks={[task]} onClose={vi.fn()} onConfirmed={onConfirmed} />);
    await screen.findByText("/downloads/a.bin");

    fireEvent.click(screen.getByRole("checkbox", { name: /同时删除文件/ }));
    fireEvent.click(screen.getByRole("button", { name: "确认移除" }));

    await waitFor(() => {
      expect(onConfirmed).toHaveBeenCalledOnce();
    });
    expect(invoke).toHaveBeenCalledWith("remove_task", {
      taskId: "task-1",
      deleteFiles: true,
    });
  });
});
