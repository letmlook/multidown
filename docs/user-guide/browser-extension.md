# 浏览器扩展指南

浏览器扩展通过 Native Messaging Host 与桌面应用通信。扩展和 Native Host 必须同时安装，且发送下载前应先启动 Multidown。

## 固定身份

- Chromium 扩展 ID：`bceackgdejcgphcbhinfgejepgoeiail`
- Firefox 扩展 ID：`multidown@letmlook`
- Native Host 名称：`com.multidown.app`

固定身份用于限制哪些扩展可以调用 Native Host。若自行修改 manifest key 或 Firefox ID，必须同步调整 Host 清单。

## Chromium（Chrome / Edge）

扩展以**已解压目录**的形式提供，不是 `.crx` 安装包。目录里包含 `icons/` 等子目录，请整体使用，不要只拷贝顶层文件。

在 Multidown 中选择“安装浏览器扩展”，应用会：

1. 注册 Native Host（`com.multidown.app`）；
2. 打开你的浏览器的扩展管理页（`chrome://extensions/` 或 `edge://extensions/`）。

之后按窗口提示启用“开发者模式”，点击“加载已解压的扩展程序”，选择应用显示的扩展文件夹路径（通常在应用数据目录的 `extension/` 下，弹窗里有“打开扩展文件夹”和“复制路径”两个按钮）。

首次启动时应用也会自动尝试一次同样的注册与引导；浏览器是否真的接受加载仍由你在扩展管理页完成。

需要把扩展交给别人时，使用弹窗里的**导出扩展 ZIP**：ZIP 的目录结构与已解压目录一致，对方解压后按上面的步骤加载即可。浏览器不能直接加载 ZIP。

从源码构建时：

```bash
npm run build:extension
npm run test:extension
```

然后在浏览器中加载 `dist-extension/unpacked`。

`package.json` 里的 `npm run sign:crx` **不会**签名或生成任何文件，它只打印 Chrome 手工“打包扩展程序”的步骤，供需要 CRX 的场景自行操作。开发与验证请直接使用 unpacked 目录；不要提交 Chrome 生成的私钥。

## Firefox

运行 `npm run build:extension` 后，Firefox 变体位于 `dist-extension/firefox-unpacked`。在 `about:debugging#/runtime/this-firefox` 中选择“临时载入附加组件”，并打开该目录的 `manifest.json`。

临时扩展会在 Firefox 重启后失效。正式、长期安装通常需要 Mozilla 签名；当前仓库提供的是开发与验证路径。

## Native Host

应用安装包应注册并携带 `multidown-native-host`。从源码运行时，需要先构建：

```bash
cargo build --release --manifest-path integration/native-host/Cargo.toml
```

随后将 Host 清单安装到浏览器规定的位置，并把清单的 `path` 指向上述可执行文件的绝对路径。具体系统位置见[平台路径参考](../reference/platform-paths.md)；低层协议说明见 [`integration/README.md`](../../integration/README.md)。

**Host 二进制是单独安装的**，只有应用的“安装浏览器扩展”动作会把它复制到浏览器的 Native Messaging 目录。因此**只升级应用不会升级浏览器里的 Host**。升级应用后如果出现连接异常，先重新执行一次“安装浏览器扩展”刷新 Host。

## 连接是怎么判定的

扩展与桌面端之间有两条固定动作：

- `test_connection`：发一次请求并要求桌面端回一个带 `multidown` 握手标记的应答，才算“已连接”；
- `open_app`：应用没运行时，请求操作系统打开 `multidown://open` 深链把它拉起来，然后在 3 秒预算内等待一次新鲜的握手成功（最坏约 5.4 秒，仍低于扩展 8 秒的等待上限）。

因为必须完成真正的握手，**残留的旧端口文件不会被误判为“应用已运行”**：即便那个端口上有别的进程在监听，没有标记也算失败。端口文件缺失或内容不是合法端口时，扩展会明确告诉你应用未运行，而不是伪装成功。

不支持的协议版本和未知动作会返回明确的错误信息，不会一直转圈或挂起。

## 捕获规则与使用

- 右键链接、页面、视频或音频可发送给 Multidown。
- 扩展可以接管浏览器下载；在“选项 → 常规”关闭“允许浏览器扩展接管下载”即可停用。
- 域名黑名单一行一个域名，子域名也会命中。
- 打开“显示开始下载对话框”时，浏览器发送的任务会先进入确认界面。

若 Native Messaging 不可用，可使用已注册的 `multidown://add?url=...` 深链作为有限兜底。深链不替代扩展的完整捕获和消息响应能力。

连接失败时按[故障排查](./troubleshooting.md#浏览器提示无法连接或-native-host-不可用)逐项检查。

## 隐私提醒

扩展的“导出日志”会把**完整下载地址与文件名**从浏览器本地的日志缓冲写成一个文本文件（`multidown-extension-logs.txt`）。扩展不读取也不保存 cookie——它没有申请 cookie 权限，发送下载时也不携带 cookie。但下载地址本身可能包含签名参数，所以导出文件仍要按敏感数据处理。这是当前的已知问题，尚未修复；导出前请自行确认文件不会外流。详见[已知限制](../reference/known-limitations.md#安全与可靠性边界)。

## 验证状态

扩展打包与 Native Messaging 协议有自动化测试覆盖（含用真实 Host 二进制对回环桌面端桩做分帧 I/O 的往返测试），但**在真实浏览器中的安装、加载、授权与握手均未在真实机器上验证过**。发布前请按[功能验收清单](../development/functionality-release-checklist.md)逐项记录证据。
