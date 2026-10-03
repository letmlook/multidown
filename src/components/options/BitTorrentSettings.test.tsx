import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../../types/download";
import { BitTorrentSettings } from "./BitTorrentSettings";

const legacySettings: AppSettings = {
  default_save_path: "",
  max_connections_per_task: 8,
  max_concurrent_tasks: 8,
  max_retries: 3,
  run_at_startup: false,
  clipboard_monitor: false,
  show_start_dialog: true,
  show_complete_dialog: true,
  duplicate_action: "ask",
  user_agent: "Multidown test",
  use_last_save_path: true,
  proxy_type: "none",
  proxy_host: "",
  proxy_port: 8080,
  notification_on_complete: true,
  notification_on_fail: true,
  timeout_secs: 30,
};

const baseProps = {
  settings: legacySettings,
  magnetStatus: null,
  magnetBusy: false,
  onSetMagnetHandler: vi.fn(async () => undefined),
};

describe("BitTorrentSettings", () => {
  it("uses_safe_defaults_for_legacy_settings", () => {
    render(<BitTorrentSettings {...baseProps} update={vi.fn()} />);

    expect(screen.getByRole("checkbox", { name: /启用 DHT/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /关闭本地服务发现/ })).toBeChecked();
    expect(screen.getByRole("spinbutton", { name: "BT 监听端口" })).toHaveValue(0);
    expect(screen.getByRole("combobox", { name: "做种策略（下载完成后）" })).toHaveValue(
      "stop",
    );
  });

  it("emits_only_the_changed_torrent_field", () => {
    const update = vi.fn();
    render(<BitTorrentSettings {...baseProps} update={update} />);

    fireEvent.click(screen.getByRole("checkbox", { name: /启用 DHT/ }));

    expect(update).toHaveBeenCalledOnce();
    expect(update).toHaveBeenCalledWith({ torrent_enable_dht: false });
  });

  it("explains_that_peer_limit_changes_rebuild_the_live_session", () => {
    render(<BitTorrentSettings {...baseProps} update={vi.fn()} />);

    expect(screen.getByText(/peer 上限变更会安全重建当前 BT 会话/)).toBeInTheDocument();
    expect(screen.queryByText(/监听端口 \/ DHT \/ 代理在重启应用后生效/)).not.toBeInTheDocument();
  });
});
