# 浏览器集成架构

## 数据流

1. 扩展从右键菜单、页面媒体或下载接管得到 URL，并应用本地捕获配置。
2. 浏览器按 Host 清单启动 `com.multidown.app`，用长度前缀 Native Messaging 帧传递 JSON。
3. Rust Native Host 读取应用数据目录中的 `native_host_port.txt`，连接 `127.0.0.1` 的随机端口。
4. 应用只接受支持的 HTTP(S)、磁力或种子输入，再次应用捕获总开关和域名黑名单。
5. 根据“显示开始下载对话框”设置，应用发出前端确认事件或直接创建任务，并把结果沿原链路返回。

## 身份约束

Chromium manifest 的固定公钥推导出扩展 ID `bceackgdejcgphcbhinfgejepgoeiail`，Host 清单的 `allowed_origins` 只允许该 origin。Firefox 构建变体使用 ID `multidown@letmlook`，并需要对应的 Native Host 允许扩展列表。

`npm run test:extension` 会从公钥重新计算 Chromium ID，防止清单身份漂移。修改身份属于兼容性和安全变更，会使既有 Host 注册失效。

## 本机转发

应用只绑定 `127.0.0.1:0`，由操作系统选择临时端口，并把端口号写入应用数据目录。Host 不对外网监听。每次连接读取一行 JSON，支持配置查询、下载和打开窗口动作，并返回单行 JSON 结果。

回环监听当前没有独立会话令牌；防护依赖本机边界、随机端口、应用数据目录权限和上游浏览器的 Native Host 身份限制。因此同一用户会话中的恶意本地进程仍是明确的剩余风险，详见[安全模型](./security.md#已知安全限制)。

## 安装与兜底

桌面安装包携带扩展资源与 Host。应用的安装引导只打开已知浏览器的扩展管理页并展示手工步骤，不关闭浏览器、不修改浏览器安全策略。开发环境可加载 unpacked 目录。

`multidown://add?url=...` 是扩展不可用时的有限兜底，并由单实例插件转交给已运行应用。它仍属于不可信外部输入，必须走统一输入解析，不能绕过确认或 URL 校验。

用户操作见[浏览器扩展指南](../user-guide/browser-extension.md)，源码入口见 [`integration/README.md`](../../integration/README.md)。
