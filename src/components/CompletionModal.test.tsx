import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { addCompletionItem } from "../types/download";
import type { CompletionItem } from "../types/download";
import { CompletionModal } from "./CompletionModal";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

function item(over: Partial<CompletionItem> = {}): CompletionItem {
  return {
    taskId: "task-1",
    filename: "a.bin",
    savePath: "/downloads/a.bin",
    ...over,
  };
}

/** 模拟 App 的队列持有者：完成事件入队，关闭弹窗清空整队 */
function QueueHarness({ enabled = true }: { enabled?: boolean }) {
  const [items, setItems] = useState<CompletionItem[]>([]);
  const complete = (next: CompletionItem) =>
    setItems((prev) => addCompletionItem(prev, next));
  return (
    <>
      <button type="button" onClick={() => complete(item())}>
        完成任务 A
      </button>
      <button
        type="button"
        onClick={() =>
          complete(item({ taskId: "task-2", filename: "b.bin", savePath: "/downloads/b.bin" }))
        }
      >
        完成任务 B
      </button>
      <button type="button" onClick={() => complete(item({ filename: "a-renamed.bin" }))}>
        任务 A 再次完成
      </button>
      <CompletionModal enabled={enabled} items={items} onClose={() => setItems([])} />
    </>
  );
}

describe("CompletionModal", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
  });

  it("lists_a_single_completion_with_its_save_path", () => {
    render(<CompletionModal enabled items={[item()]} onClose={vi.fn()} />);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("下载完成")).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
    expect(screen.getByText("a.bin")).toBeInTheDocument();
    expect(screen.getByText("/downloads/a.bin")).toBeInTheDocument();
  });

  it("aggregates_a_burst_into_one_modal_without_losing_items", async () => {
    render(<QueueHarness />);

    fireEvent.click(screen.getByRole("button", { name: "完成任务 A" }));
    fireEvent.click(screen.getByRole("button", { name: "完成任务 B" }));
    fireEvent.click(screen.getByRole("button", { name: "任务 A 再次完成" }));

    // 一个弹窗承载全部未确认条目，重复的 taskId 不产生第二行
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getByText("2 个任务已完成")).toBeInTheDocument();
    expect(await screen.findByText("a-renamed.bin")).toBeInTheDocument();
    expect(screen.getByText("b.bin")).toBeInTheDocument();
  });

  it("replaces_the_existing_row_when_the_same_task_finishes_again", () => {
    const first = addCompletionItem([], item({ filename: "old.bin", savePath: null }));
    const second = addCompletionItem(first, item({ filename: "fresh.bin", savePath: "/downloads/fresh.bin" }));

    expect(second).toHaveLength(1);
    expect(second[0]).toEqual(item({ filename: "fresh.bin", savePath: "/downloads/fresh.bin" }));
  });

  it("close_clears_the_whole_queue", async () => {
    render(<QueueHarness />);
    fireEvent.click(screen.getByRole("button", { name: "完成任务 A" }));
    fireEvent.click(screen.getByRole("button", { name: "完成任务 B" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(2);

    fireEvent.click(screen.getByRole("button", { name: "关闭" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    // 队列已清空：再次完成只列出新的一条，而不是累积旧的两条
    fireEvent.click(screen.getByRole("button", { name: "完成任务 A" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
    expect(screen.getByText("a.bin")).toBeInTheDocument();
  });

  it("renders_nothing_when_the_complete_dialog_setting_is_off", () => {
    render(<QueueHarness enabled={false} />);

    fireEvent.click(screen.getByRole("button", { name: "完成任务 A" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("a.bin")).not.toBeInTheDocument();
  });

  it("never_renders_an_empty_dialog", () => {
    render(<CompletionModal enabled items={[]} onClose={vi.fn()} />);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("opens_the_file_with_the_task_save_path", async () => {
    render(<CompletionModal enabled items={[item()]} onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "打开文件：a.bin" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("open_file", { path: "/downloads/a.bin" });
    });
  });

  it("opens_the_containing_directory_with_the_task_save_path", async () => {
    render(<CompletionModal enabled items={[item()]} onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "打开所在目录：a.bin" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("open_folder", { path: "/downloads/a.bin" });
    });
  });

  it("marks_an_unknown_save_path_and_disables_both_open_actions", () => {
    render(<CompletionModal enabled items={[item({ savePath: null })]} onClose={vi.fn()} />);

    expect(screen.getByText(/保存路径未知/)).toBeInTheDocument();
    const openFile = screen.getByRole("button", { name: "打开文件：a.bin" });
    const openFolder = screen.getByRole("button", { name: "打开所在目录：a.bin" });
    expect(openFile).toBeDisabled();
    expect(openFolder).toBeDisabled();

    fireEvent.click(openFile);
    fireEvent.click(openFolder);
    expect(invoke).not.toHaveBeenCalledWith("open_file", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("open_folder", expect.anything());
  });

  it("surfaces_an_open_failure_without_losing_the_list", async () => {
    invoke.mockImplementation((command: string) =>
      command === "open_file" ? Promise.reject("文件不存在") : Promise.resolve(undefined)
    );
    render(<CompletionModal enabled items={[item()]} onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "打开文件：a.bin" }));

    expect(await screen.findByText(/文件不存在/)).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });
});
