import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OptionsModal } from "./OptionsModal";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("OptionsModal scheduling contracts", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((command: string) => {
      if (command === "get_settings") return Promise.resolve({});
      if (command === "get_schedule_state") return Promise.resolve({ enabled: true });
      if (command === "get_schedule_tasks") return Promise.resolve([]);
      if (command === "create_schedule_task") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
  });

  it("sends one-shot date separately while recurrence remains tagged IPC JSON", async () => {
    render(<OptionsModal open onClose={vi.fn()} initialTab="schedule" />);
    const create = await screen.findByRole("button", { name: /新建调度任务/ });
    fireEvent.click(create);

    const inputs = screen.getAllByRole("textbox");
    fireEvent.change(inputs[0], { target: { value: "Only once" } });
    fireEvent.change(screen.getAllByRole("combobox")[1], { target: { value: "once" } });
    fireEvent.change(screen.getAllByRole("textbox")[1], { target: { value: "2026-10-04" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("create_schedule_task", {
        name: "Only once",
        scheduleType: "start_download",
        recurrence: { type: "once", date: "2026-10-04" },
        scheduledDate: "2026-10-04",
        startTime: "08:00",
        endTime: undefined,
        speedLimitKbps: undefined,
      }),
    );
  });
});
