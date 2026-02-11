# Multidown 完整测试流程

本指南详细说明如何整合和测试 Multidown 的所有组件：浏览器扩展、本地消息主机和主程序。

## 一、组件编译

### 1. 编译浏览器扩展

```bash
npm run build:extension
```

生成文件：
- `dist-extension/unpacked/` - 解压后的扩展目录（用于开发者模式）
- `dist-extension/multidown-extension.zip` - 扩展的ZIP包

### 2. 编译本地消息主机

```bash
cd integration/native-host && cargo build --release
```

生成文件：
- `integration/native-host/target/release/multidown-native-host.exe` - 本地消息主机可执行文件

### 3. 编译主程序

```bash
npm run tauri:build
```

生成文件：
- `src-tauri/target/release/multidown.exe` - 主程序可执行文件
- `src-tauri/target/release/bundle/` - 安装包文件

## 二、完整安装流程

### 步骤 1：安装主程序

1. **运行安装包**：
   - 执行 `src-tauri/target/release/bundle/nsis/MultiDown_0.1.0_x64-setup.exe`
   - 按照安装向导完成安装

2. **或直接运行可执行文件**：
   - 直接运行 `src-tauri/target/release/multidown.exe`

### 步骤 2：安装浏览器扩展

1. **打开浏览器扩展管理页面**：
   - Chrome: `chrome://extensions/`
   - Edge: `edge://extensions/`

2. **启用开发者模式**：
   - 点击页面右上角的"开发者模式"开关

3. **加载扩展**：
   - 点击"加载已解压的扩展程序"
   - 选择目录：`dist-extension/unpacked`

4. **验证安装**：
   - 扩展应该显示在扩展列表中
   - 浏览器右上角应该出现 Multidown 扩展图标

### 步骤 3：注册本地消息主机

#### Windows 系统

1. **创建注册表项**：
   - 打开注册表编辑器 (`regedit.exe`)
   - 导航到：`HKEY_CURRENT_USER\Software\Google\Chrome\NativeMessagingHosts\com.multidown.app`
   - 设置默认值为：`C:\Program Files\MultiDown\native-host\com.multidown.app.json`

2. **复制配置文件**：
   - 创建目录：`C:\Program Files\MultiDown\native-host\`
   - 复制文件：`integration/extension/com.multidown.app.json` 到该目录
   - 复制文件：`integration/native-host/target/release/multidown-native-host.exe` 到该目录

3. **修改配置文件**：
   - 编辑 `com.multidown.app.json`
   - 将 `"path"` 字段设置为：`"C:\Program Files\MultiDown\native-host\multidown-native-host.exe"`

#### 其他系统

参考 `src/lib.rs` 中的 `register_native_host` 函数实现。

## 三、功能测试

### 测试 1：右键菜单下载

1. **打开任意网页**：
   - 例如：`https://example.com`

2. **测试右键菜单**：
   - 右键点击任何链接
   - 选择"使用 Multidown 下载链接"

3. **验证结果**：
   - Multidown 主窗口应该自动打开
   - 下载任务应该被添加到任务列表
   - 下载应该开始执行

### 测试 2：视频下载

1. **打开视频网站**：
   - 例如：YouTube、Bilibili 等

2. **测试视频右键菜单**：
   - 右键点击视频
   - 选择"使用 Multidown 下载视频"

3. **验证结果**：
   - Multidown 主窗口应该自动打开
   - 视频下载任务应该被添加

### 测试 3：音频下载

1. **打开音频网站**：
   - 例如：音频播放网站

2. **测试音频右键菜单**：
   - 右键点击音频
   - 选择"使用 Multidown 下载音频"

3. **验证结果**：
   - Multidown 主窗口应该自动打开
   - 音频下载任务应该被添加

### 测试 4：页面下载

1. **打开任意网页**：
   - 例如：`https://example.com`

2. **测试页面右键菜单**：
   - 在页面空白处右键点击
   - 选择"使用 Multidown 下载此页面"

3. **验证结果**：
   - Multidown 主窗口应该自动打开
   - 页面下载任务应该被添加

## 四、通信机制测试

### 测试 1：扩展与本地消息主机通信

1. **检查本地消息主机日志**：
   - 打开命令提示符
   - 运行：`C:\Program Files\MultiDown\native-host\multidown-native-host.exe`
   - 观察控制台输出

2. **触发扩展操作**：
   - 在浏览器中使用扩展的右键菜单
   - 观察本地消息主机的输出

### 测试 2：本地消息主机与主程序通信

1. **启动主程序**：
   - 运行 `multidown.exe`

2. **检查主程序日志**：
   - 主程序应该在后台监听 TCP 连接
   - 检查应用数据目录中的端口文件：
     - `%APPDATA%\com.multidown.app\native_host_port.txt`

3. **触发下载操作**：
   - 在浏览器中使用扩展下载功能
   - 观察主程序是否接收到下载请求

## 五、常见问题排查

### 1. 扩展无法连接本地消息主机

**症状**：
- 右键菜单点击后无反应
- 浏览器控制台显示错误："无法连接 Native Host"

**排查步骤**：
1. 检查本地消息主机是否正确注册
2. 验证注册表项是否正确
3. 检查配置文件路径是否正确
4. 手动运行本地消息主机检查是否有错误

### 2. 本地消息主机无法连接主程序

**症状**：
- 扩展显示"Multidown 未运行或未就绪"
- 本地消息主机日志显示连接错误

**排查步骤**：
1. 确保主程序已启动
2. 检查端口文件是否存在：`%APPDATA%\com.multidown.app\native_host_port.txt`
3. 验证端口文件中的端口号是否正确
4. 检查防火墙是否阻止连接

### 3. 主窗口不自动打开

**症状**：
- 扩展触发下载后，主窗口不显示

**排查步骤**：
1. 检查主程序是否已安装
2. 验证本地消息主机配置是否正确
3. 检查主程序是否在后台运行
4. 手动打开主程序后再次测试

### 4. 下载任务不添加

**症状**：
- 扩展操作后，主窗口打开但无任务添加

**排查步骤**：
1. 检查网络连接
2. 验证 URL 是否有效
3. 检查主程序日志
4. 测试直接在主程序中添加相同 URL

## 六、调试技巧

### 1. 浏览器扩展调试

1. **打开扩展背景页**：
   - 在扩展管理页面，点击扩展的"详细信息"
   - 点击"背景页"
   - 在开发者工具中查看控制台输出

2. **查看网络请求**：
   - 在开发者工具的"网络"标签页查看网络请求

### 2. 本地消息主机调试

1. **手动运行**：
   - 在命令提示符中直接运行本地消息主机
   - 观察控制台输出

2. **检查通信数据**：
   - 使用网络抓包工具查看 TCP 通信

### 3. 主程序调试

1. **开发模式运行**：
   ```bash
   npm run tauri:dev
   ```

2. **查看日志**：
   - 检查应用数据目录中的日志文件

## 七、测试环境要求

### 软件要求

- **操作系统**：Windows 10/11、macOS 10.15+、Linux
- **浏览器**：Chrome 90+、Edge 90+
- **Node.js**：18+
- **Rust**：1.70+
- **网络连接**：稳定的互联网连接

### 硬件要求

- **CPU**：至少 2 核心
- **内存**：至少 4GB
- **磁盘空间**：至少 100MB 可用空间

## 八、测试结果记录

| 测试项 | 预期结果 | 实际结果 | 状态 | 备注 |
|--------|----------|----------|------|------|
| 扩展安装 | 扩展成功安装 | | | |
| 右键菜单显示 | 显示下载选项 | | | |
| 链接下载 | 成功添加任务 | | | |
| 视频下载 | 成功添加任务 | | | |
| 音频下载 | 成功添加任务 | | | |
| 页面下载 | 成功添加任务 | | | |
| 窗口自动打开 | 主窗口自动显示 | | | |
| 下载执行 | 下载正常进行 | | | |
| 通信链路 | 各组件通信正常 | | | |

---

## 九、总结

本测试流程涵盖了 Multidown 的完整功能测试，包括：

1. **组件编译**：确保所有组件正确构建
2. **系统安装**：完整的安装和配置过程
3. **功能测试**：验证所有下载功能
4. **通信测试**：确保组件间通信正常
5. **问题排查**：常见问题的解决方法

通过遵循本指南，您可以全面测试 Multidown 的所有功能，确保其与 IDM 对齐的用户体验和通信机制正常工作。

---

**注意**：本测试流程适用于开发和测试环境。在生产环境中，建议使用正式签名的扩展和安装包。
