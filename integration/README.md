# 浏览器集成（阶段三）

## 1. Native Messaging Host（本机通信宿主）

用于接收 Chrome 扩展发来的下载链接并转发给 Multidown 主程序。

### 编译

```bash
cd integration/native-host
cargo build --release
```

生成的可执行文件：`target/release/multidown-native-host.exe`（Windows）或 `multidown-native-host`（macOS/Linux）。

### 注册到 Chrome

Chrome 通过“宿主清单”找到该可执行文件，需在系统中注册一次。

**Windows（当前用户）：**

1. 将 `extension/com.multidown.app.json` 复制到某固定位置（如 `%LOCALAPPDATA%\Multidown\`）。
2. 在 JSON 中把 `MULTIDOWN_NATIVE_HOST_PATH` 改为本机 Host 可执行文件的**绝对路径**（注意反斜杠要写成 `\\`）。
3. 把 `EXTENSION_ID_PLACEHOLDER` 换成你的扩展 ID（在 `chrome://extensions` 中加载解压的扩展后可见）。
4. 注册表添加项：
   - 路径：`HKEY_CURRENT_USER\Software\Google\Chrome\NativeMessagingHosts\com.multidown.app`
   - 默认值：上述 JSON 文件的完整路径（例如 `C:\Users\你的用户名\AppData\Local\Multidown\com.multidown.app.json`）。

**macOS / Linux：**

- 将 `com.multidown.app.json` 放到 Chrome 约定的 Native Messaging Host 目录（见 [Chrome 文档](https://developer.chrome.com/docs/apps/nativeMessaging/#native-messaging-host-location)），并修改其中的 `path` 和 `allowed_origins`（扩展 ID）。

## 2. Chrome 扩展

提供右键菜单「使用 Multidown 下载链接」/「使用 Multidown 下载此页面」。

### 安装（开发）

1. 编译并注册上述 Native Host。
2. 在 Chrome 打开 `chrome://extensions`，开启「开发者模式」。
3. 点击「加载已解压的扩展程序」，选择本仓库下的 `integration/extension` 目录。
4. 记下扩展 ID，填入 `com.multidown.app.json` 的 `allowed_origins` 后重新注册 Native Host。

### 使用

- 在链接上右键 →「使用 Multidown 下载链接」：将该链接发送给 Multidown 加入下载。
- 在页面空白处右键 →「使用 Multidown 下载此页面」：将当前页面 URL 发送给 Multidown。

**注意：** 主程序 Multidown 需先启动，否则 Native Host 会提示“Multidown 未运行或未就绪”。

## 3. 主程序侧

主程序启动时会：

- 在 `127.0.0.1` 上监听一个随机端口；
- 将端口号写入应用数据目录下的 `native_host_port.txt`（与 Tauri `app_data_dir` 一致，如 Windows 下 `%APPDATA%\com.multidown.app\`）。

Native Host 会读取该文件并连接对应端口，将扩展发来的 URL 转发给主程序，由主程序创建并开始下载任务。

### 主程序与 Native Host 的 TCP 协议

- **请求**（Native Host → 主程序）：一行 JSON，以换行结尾，如 `{"url":"https://example.com/file.zip"}\n`。
- **响应**（主程序 → Native Host）：一行 JSON，以换行结尾。成功：`{"ok":true}\n`；失败：`{"ok":false,"error":"错误信息"}\n`。
- Native Host 根据主程序响应再向 Chrome 返回成功或失败，扩展侧可据此提示用户。
