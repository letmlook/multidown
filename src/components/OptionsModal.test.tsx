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

  it("selects lowercase weekdays before creating a weekly schedule", async () => {
    render(<OptionsModal open onClose={vi.fn()} initialTab="schedule" />);
    fireEvent.click(await screen.findByRole("button", { name: /新建调度任务/ }));
    fireEvent.change(screen.getAllByRole("textbox")[0], { target: { value: "Weekdays" } });
    fireEvent.change(screen.getAllByRole("combobox")[1], { target: { value: "weekly" } });
    fireEvent.click(screen.getByRole("checkbox", { name: "周一" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "周三" }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("create_schedule_task", {
        name: "Weekdays",
        scheduleType: "start_download",
        recurrence: { type: "weekly", days: ["mon", "wed"] },
        scheduledDate: undefined,
        startTime: "08:00",
        endTime: undefined,
        speedLimitKbps: undefined,
      }),
    );
  });

  it("renders a returned one-shot scheduled date", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_settings") return Promise.resolve({});
      if (command === "get_schedule_state") return Promise.resolve({ enabled: true });
      if (command === "get_schedule_tasks") {
        return Promise.resolve([
          {
            id: "once",
            name: "One-shot",
            enabled: true,
            schedule_type: "pause_all",
            recurrence: { type: "once" },
            scheduled_date: "2026-10-04",
            start_time: "08:00",
          },
        ]);
      }
      return Promise.resolve(undefined);
    });

    render(<OptionsModal open onClose={vi.fn()} initialTab="schedule" />);
    expect(await screen.findByText("一次 (2026-10-04)")).toBeInTheDocument();
  });
});
