import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface InstallExtensionModalProps {
  open: boolean;
  onClose: () => void;
}

export function InstallExtensionModal({ open, onClose }: InstallExtensionModalProps) {
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const doWithPath = async (
    action: "open" | "copy"
  ): Promise<void> => {
    setError(null);
    setCopied(false);
    try {
      const path = await invoke<string>("get_browser_extension_path");
      if (action === "open") {
        await invoke("open_folder", { path });
      } else {
        await invoke("write_clipboard_text", { text: path });
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const installExtension = async (): Promise<void> => {
    setError(null);
    try {
      await invoke("install_browser_extension");
    } catch (e) {
      setError(String(e));
    }
  };

  const packageExtension = async (): Promise<void> => {
    setError(null);
    try {
      const zipPath = await invoke<string>("package_browser_extension");
      await invoke("open_folder", { path: zipPath });
    } catch (e) {
      setError(String(e));
    }
  };

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ maxWidth: 480 }}>
        <div className="modal-title">安装浏览器扩展</div>
        <div className="modal-body" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <p style={{ margin: 0, color: "var(--text-secondary, #666)" }}>
            安装后可在浏览器中通过右键菜单「使用 Multidown 下载链接」将链接发送到本程序。
          </p>
          <ol style={{ margin: "8px 0", paddingLeft: 20 }}>
            <li>打开 Chrome（或 Edge），在地址栏输入 <strong>chrome://extensions</strong></li>
            <li>打开右上角「开发者模式」</li>
            <li>点击「加载已解压的扩展程序」</li>
            <li>选择下方「打开扩展文件夹」后显示的文件夹</li>
          </ol>
          <p style={{ margin: 0, fontSize: 12, color: "var(--text-secondary, #666)" }}>
            安装扩展后还需注册 Native Host，详见 <code>integration/README.md</code>。
          </p>
          {error && (
            <p style={{ margin: 0, color: "var(--error, #c00)", fontSize: 13 }}>{error}</p>
          )}
        </div>
        <div className="modal-footer" style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <button
            type="button"
            className="btn btn-primary"
            onClick={installExtension}
          >
            自动安装扩展
          </button>
          <button
            type="button"
            className="btn"
            onClick={packageExtension}
          >
            打包扩展
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => doWithPath("open")}
          >
            打开扩展文件夹
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => doWithPath("copy")}
          >
            {copied ? "已复制" : "复制扩展路径"}
          </button>
          <button type="button" className="btn" onClick={onClose}>
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
