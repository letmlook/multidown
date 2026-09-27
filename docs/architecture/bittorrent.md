# BitTorrent 架构

## 输入与任务占位

`torrent/detect.rs` 识别磁力链接、`.torrent` 路径或 URL，校验 v1 info hash，并解析 `dn`、`tr`、`so` 等磁力参数。`.torrent` 元信息可以本地解析；磁力链接通常需要网络发现元数据。

为了不让 UI 阻塞，裸磁力可先建立 `TaskKind::Torrent` 占位任务。占位记录输入和可用的显示名，状态标记元数据尚未就绪；解析完成后再补充 info hash、文件列表和动态总大小。

## librqbit 会话

`torrent/engine.rs` 将应用设置转换为 librqbit 会话配置，包括 DHT、本地发现、监听端口、下载/上传限速、peer 上限和 SOCKS5 代理。会话状态放在应用数据目录的 `torrent-session/`，用于 fastresume 和重启恢复。

每个 Multidown 任务与一个种子会话关联。引擎轮询统计并把已下载、已上传、peer 数、元数据状态和错误同步回统一任务模型，因此前端无需直接依赖 librqbit 类型。

## 文件选择与完成

解析元数据后，文件索引列表成为引擎侧权威选择；创建时和运行中修改都会传给会话并持久化。任务总大小按已选文件计算，未选择文件不应被当作待完成内容。

数据下载完成后，任务是否立即停止取决于做种策略：停止、目标分享率、目标时长或持续做种。完成下载与完成做种是两个不同阶段；UI 的完成语义应同时考虑策略状态。

## 隐私与代理

BT SOCKS5 代理独立于 HTTP 代理。代理存在时配置层强制关闭 DHT，并强制禁用本地发现，防止 UDP 发现路径绕过代理；tracker 和 peer TCP/uTP 能力仍受 librqbit 与代理支持范围限制。该机制降低泄漏面，但不构成匿名保证。

## 恢复与限制

统一任务 JSON 保留输入、info hash、内嵌 metainfo、所选文件、元数据状态和上传量；librqbit 会话目录保留协议级恢复状态。两者缺一时仍应尽量重建，但可能重新解析元数据或校验文件。

当前不支持 web seed、MSE/PE 连接加密和顺序下载。公网 DHT 冒烟测试默认忽略，详见[测试指南](../development/testing.md#rust-应用)。
