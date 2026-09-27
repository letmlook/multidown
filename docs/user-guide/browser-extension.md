# 浏览器扩展指南

浏览器扩展通过 Native Messaging Host 与桌面应用通信。扩展和 Native Host 必须同时安装，且发送下载前应先启动 Multidown。

## 固定身份

- Chromium 扩展 ID：`bceackgdejcgphcbhinfgejepgoeiail`
- Firefox 扩展 ID：`multidown@letmlook`
- Native Host 名称：`com.multidown.app`

固定身份用于限制哪些扩展可以调用 Native Host。若自行修改 manifest key 或 Firefox ID，必须同步调整 Host 清单。

## Chromium（Chrome / Edge）

稳定安装包会携带扩展资源和 Native Host。可在 Multidown 中选择“安装浏览器扩展”，按窗口提示打开 `chrome://extensions/` 或 `edge://extensions/`，启用开发者模式后加载应用提供的扩展目录。

从源码构建时：

```bash
npm run build:extension
npm run test:extension
```

然后在浏览器中加载 `dist-extension/unpacked`。请勿安装来源不明的 CRX；开发模式下“加载已解压的扩展程序”更容易检查实际内容。

## Firefox

运行 `npm run build:extension` 后，Firefox 变体位于 `dist-extension/firefox-unpacked`。在 `about:debugging#/runtime/this-firefox` 中选择“临时载入附加组件”，并打开该目录的 `manifest.json`。

临时扩展会在 Firefox 重启后失效。正式、长期安装通常需要 Mozilla 签名；当前仓库提供的是开发与验证路径。

## Native Host

应用安装包应注册并携带 `multidown-native-host`。从源码运行时，需要先构建：

```bash
cargo build --release --manifest-path integration/native-host/Cargo.toml
```

随后将 Host 清单安装到浏览器规定的位置，并把清单的 `path` 指向上述可执行文件的绝对路径。具体系统位置见[平台路径参考](../README.md#我负责发布维护)；当前低层协议说明保留在 [`integration/README.md`](../../integration/README.md)。

## 捕获规则与使用

- 右键链接、页面、视频或音频可发送给 Multidown。
- 扩展可以接管浏览器下载；在“选项 → 常规”关闭“允许浏览器扩展接管下载”即可停用。
- 域名黑名单一行一个域名，子域名也会命中。
- 打开“显示开始下载对话框”时，浏览器发送的任务会先进入确认界面。

若 Native Messaging 不可用，可使用已注册的 `multidown://add?url=...` 深链作为有限兜底。深链不替代扩展的完整捕获和消息响应能力。

连接失败时按[故障排查](./troubleshooting.md#浏览器提示无法连接或-native-host-不可用)逐项检查。
