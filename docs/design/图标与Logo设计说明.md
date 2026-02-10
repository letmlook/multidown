# Multidown 图标与 Logo 设计说明

## 设计概念

- **应用图标**：圆角方底 + 向下箭头拆成 4 条平行斜条，表达「多连接 / 多线程下载」。
- **主色**：青色系（`#06b6d4` ~ `#0e7490`），偏工具/效率感，易识别。
- **Logo**：图标 + 文字「Multidown」，用于 README、关于页、启动图等。

## 文件说明

| 文件 | 用途 |
|------|------|
| `icon.svg` | 应用图标源文件，512×512，可导出为 PNG/ICO/ICNS |
| `logo.svg` | 品牌 Logo 源文件，360×80，图标 + 文字 |
| `icon-preview.html` | 本地预览图标与 Logo（用浏览器打开） |

## 导出为 Tauri 所需格式

Tauri 需要：`icon.png`、`32x32.png`、`64x64.png`、`128x128.png`、`128x128@2x.png`、`icon.ico`、`icon.icns`。

### 方法一：在线 / 本地工具

1. 用浏览器打开 `icon-preview.html`，或直接用 [SVG 转 PNG](https://cloudconvert.com/svg-to-png) 等工具。
2. 将 `icon.svg` 导出为 **1024×1024** 或 **512×512** 的 PNG，作为主源图。
3. 用 [icoconvert](https://icoconvert.com/) 或 [realfavicongenerator](https://realfavicongenerator.net/) 等生成 32/64/128/256 及 ico、icns。

### 方法二：ImageMagick（命令行）

```bash
# 需先安装 ImageMagick
# 从 icon.svg 导出各尺寸 PNG
magick -background none -density 1024 docs/design/icon.svg -resize 1024x1024 src-tauri/icons/icon.png
magick docs/design/icon.svg -resize 32x32 src-tauri/icons/32x32.png
magick docs/design/icon.svg -resize 64x64 src-tauri/icons/64x64.png
magick docs/design/icon.svg -resize 128x128 src-tauri/icons/128x128.png
magick docs/design/icon.svg -resize 256x256 src-tauri/icons/128x128@2x.png
# ICO（多尺寸合一）
magick docs/design/icon.svg -define icon:auto-resize=256,128,64,48,32,16 src-tauri/icons/icon.ico
```

### 方法三：Tauri 官方

使用 [Tauri 的 icon 生成](https://v2.tauri.app/develop/customization/icons/)：准备好一张 1024×1024 或 512×512 的 PNG，运行：

```bash
npm run tauri icon path/to/icon.png
```

会自动生成 `src-tauri/icons/` 下所需全部尺寸及 ico、icns。

## 颜色规范

- 主色：`#06b6d4`（Cyan 500）
- 深色：`#0e7490`（Cyan 700），用于渐变或对比
- 文字（Logo）：`#0f172a`（Slate 900）

## 使用场景

- **窗口 / 任务栏 / 安装包**：使用 `icon.png`、`icon.ico`、`icon.icns` 及各尺寸 PNG。
- **README、关于页、启动图**：使用 `logo.svg` 或导出的 `logo.png`（浅色背景）。
- 深色背景可导出 Logo 反色版（文字改为白色，图标保持或微调）。

## 窗口/任务栏图标不更新时

图标在**编译时**嵌入 exe，必须用**新构建**的可执行文件才会显示新图标。

1. **重新构建**  
   - 开发：先关掉正在运行的 Multidown，再执行 `npm run tauri dev`（会用当前 `src-tauri/icons/icon.ico` 重新编译并运行）。  
   - 或只编译：`cd src-tauri && cargo build`，然后运行 `src-tauri/target/debug/multidown.exe`（需先在前台运行 `npm run dev` 提供前端）。

2. **确认未用旧 exe**  
   确保启动的是本次构建生成的 exe（例如 `src-tauri/target/debug/multidown.exe` 或 release 目录下的），不要用之前复制到别处的旧文件。

3. **Windows 图标缓存**  
   若仍显示旧图标，可尝试：关闭所有 Multidown 窗口 → 任务管理器中结束「Windows 资源管理器」→ 文件 → 运行新任务 → 输入 `explorer` 回车，或重启电脑后再打开新构建的 exe。
