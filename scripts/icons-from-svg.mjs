#!/usr/bin/env node
/**
 * 从 docs/design/icon.svg 生成 1024x1024 PNG，供 tauri icon 使用
 * 运行: node scripts/icons-from-svg.mjs
 * 然后: npm run tauri icon src-tauri/icons/icon-1024.png
 */
import { readFileSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const svgPath = join(root, "docs", "design", "icon.svg");
const outPath = join(root, "src-tauri", "icons", "icon-1024.png");

const sharp = await import("sharp").catch(() => null);
if (!sharp) {
  console.error("请先安装 sharp: npm i -D sharp");
  process.exit(1);
}

const svg = readFileSync(svgPath);
await sharp.default(svg)
  .resize(1024, 1024)
  .png()
  .toFile(outPath);

console.log("已生成:", outPath);
console.log("请运行: npm run tauri icon src-tauri/icons/icon-1024.png");
