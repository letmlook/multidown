// build-extension.mjs - Build and package the Multidown Chrome Extension
import fs from 'fs';
import path from 'path';
import zlib from 'zlib';
import { fileURLToPath } from 'url';
import { execSync } from 'child_process';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');
const extensionDir = path.resolve(projectRoot, 'integration', 'extension');
const outputDir = path.resolve(projectRoot, 'dist-extension');

// Ensure output directory exists
if (!fs.existsSync(outputDir)) {
  fs.mkdirSync(outputDir, { recursive: true });
}

function copyDir(src, dest) {
  if (!fs.existsSync(dest)) fs.mkdirSync(dest, { recursive: true });
  const entries = fs.readdirSync(src, { withFileTypes: true });
  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      copyDir(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

// ─── 图标必须先生成：manifest 引用的 icons/*.png 缺失时打包即产出坏扩展 ───
const iconsDir = path.join(extensionDir, 'icons');
if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

// Create simple SVG-based PNG icons using ImageMagick or fallback
function createIcon(size) {
  const iconPath = path.join(iconsDir, `icon${size}.png`);
  if (fs.existsSync(iconPath)) return;

  // Try ImageMagick first
  try {
    execSync(`convert -size ${size}x${size} xc:transparent -fill "#00f5ff" -draw "roundrectangle 2,2,${size-3},${size-3},4,4" -fill "#0a0a0f" -gravity center -pointsize ${Math.floor(size * 0.5)} -annotate 0 "M" "${iconPath}"`, { stdio: 'pipe' });
    console.log(`   Created icon: ${iconPath}`);
    return;
  } catch (e) { /* fall through */ }

  // Fallback: create a minimal 1x1 transparent PNG (Chrome will show default)
  // PNG header: 8-byte minimal valid PNG
  const pngHeader = Buffer.from([
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, // RGBA
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, // IDAT chunk
    0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, // compressed data
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, // end
    0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, // IEND chunk
    0x42, 0x60, 0x82
  ]);
  fs.writeFileSync(iconPath, pngHeader);
  console.log(`   Created placeholder icon: ${iconPath}`);
}

// Generate placeholder icons
[16, 48, 128].forEach(size => {
  createIcon(size);
});

// Copy extension files to dist folder (unpacked), including nested directories such as icons/
const unpackedDir = path.resolve(outputDir, 'unpacked');
if (fs.existsSync(unpackedDir)) {
  fs.rmSync(unpackedDir, { recursive: true });
}
fs.mkdirSync(unpackedDir, { recursive: true });

console.log('📦 Copying extension files to unpacked directory...');
copyDir(extensionDir, unpackedDir);
console.log(`   Copied to: ${unpackedDir}`);

// ─── Firefox 变体：MV3 in Firefox 用 event page（background.scripts），需 gecko ID ───
const firefoxDir = path.resolve(outputDir, 'firefox-unpacked');
if (fs.existsSync(firefoxDir)) {
  fs.rmSync(firefoxDir, { recursive: true });
}
fs.mkdirSync(firefoxDir, { recursive: true });
copyDir(unpackedDir, firefoxDir);

const ffManifestPath = path.join(firefoxDir, 'manifest.json');
const ffManifest = JSON.parse(fs.readFileSync(ffManifestPath, 'utf-8'));
delete ffManifest.key;
delete ffManifest.background.service_worker;
ffManifest.background = { scripts: ['background.js'] };
ffManifest.browser_specific_settings = {
  gecko: {
    id: 'multidown@letmlook',
    strict_min_version: '115.0'
  }
};
fs.writeFileSync(ffManifestPath, JSON.stringify(ffManifest, null, 2));
console.log(`🦊 Firefox variant written to: ${firefoxDir}`);

// ─── ZIP 打包：仅用 Node 内置能力，产物路径与解压目录保持一致 ───
const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let index = 0; index < 256; index += 1) {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }
    table[index] = value >>> 0;
  }
  return table;
})();

/** 列出目录下所有文件，archivePath 始终使用 `/` 分隔并与解压目录相对路径一致。 */
export function collectExtensionFiles(root) {
  const files = [];
  const walk = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const absolutePath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        walk(absolutePath);
      } else if (entry.isFile()) {
        files.push({
          absolutePath,
          archivePath: path.relative(root, absolutePath).split(path.sep).join('/')
        });
      }
    }
  };
  walk(root);
  return files.sort((left, right) => (left.archivePath < right.archivePath ? -1 : 1));
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (let index = 0; index < buffer.length; index += 1) {
    crc = CRC_TABLE[(crc ^ buffer[index]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

// 固定时间戳（2026-01-01），保证同样输入得到同样的 ZIP 字节
const DOS_TIME = 0;
const DOS_DATE = ((2026 - 1980) << 9) | (1 << 5) | 1;
const UTF8_FLAG = 0x800;

/** 用 Node 内置 zlib 写出一个真实可读的 ZIP（deflate，压缩无收益时回退 store）。 */
export function createZip(entries, zipPath) {
  const localChunks = [];
  const centralChunks = [];
  let offset = 0;

  for (const { absolutePath, archivePath } of entries) {
    const raw = fs.readFileSync(absolutePath);
    const deflated = zlib.deflateRawSync(raw, { level: 9 });
    const useDeflate = deflated.length < raw.length;
    const payload = useDeflate ? deflated : raw;
    const method = useDeflate ? 8 : 0;
    const name = Buffer.from(archivePath, 'utf8');
    const checksum = crc32(raw);

    const localHeader = Buffer.alloc(30);
    localHeader.writeUInt32LE(0x04034b50, 0);
    localHeader.writeUInt16LE(20, 4);
    localHeader.writeUInt16LE(UTF8_FLAG, 6);
    localHeader.writeUInt16LE(method, 8);
    localHeader.writeUInt16LE(DOS_TIME, 10);
    localHeader.writeUInt16LE(DOS_DATE, 12);
    localHeader.writeUInt32LE(checksum, 14);
    localHeader.writeUInt32LE(payload.length, 18);
    localHeader.writeUInt32LE(raw.length, 22);
    localHeader.writeUInt16LE(name.length, 26);
    localHeader.writeUInt16LE(0, 28);
    localChunks.push(localHeader, name, payload);

    const centralHeader = Buffer.alloc(46);
    centralHeader.writeUInt32LE(0x02014b50, 0);
    centralHeader.writeUInt16LE(20, 4);
    centralHeader.writeUInt16LE(20, 6);
    centralHeader.writeUInt16LE(UTF8_FLAG, 8);
    centralHeader.writeUInt16LE(method, 10);
    centralHeader.writeUInt16LE(DOS_TIME, 12);
    centralHeader.writeUInt16LE(DOS_DATE, 14);
    centralHeader.writeUInt32LE(checksum, 16);
    centralHeader.writeUInt32LE(payload.length, 20);
    centralHeader.writeUInt32LE(raw.length, 24);
    centralHeader.writeUInt16LE(name.length, 28);
    centralHeader.writeUInt16LE(0, 30);
    centralHeader.writeUInt16LE(0, 32);
    centralHeader.writeUInt16LE(0, 34);
    centralHeader.writeUInt16LE(0, 36);
    centralHeader.writeUInt32LE(0, 38);
    centralHeader.writeUInt32LE(offset, 42);
    centralChunks.push(centralHeader, name);

    offset += localHeader.length + name.length + payload.length;
  }

  const central = Buffer.concat(centralChunks);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(central.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);

  fs.writeFileSync(zipPath, Buffer.concat([...localChunks, central, end]));
  return zipPath;
}

const zipPath = path.resolve(outputDir, 'multidown-extension.zip');
console.log('\n📁 Creating ZIP package...');
createZip(collectExtensionFiles(unpackedDir), zipPath);
console.log(`   ZIP created: ${zipPath}`);

console.log('\n==========================================');
console.log('   Extension Build Complete!');
console.log('==========================================');
console.log('\n📦 Extension packages:');
console.log(`   📂 Unpacked (Chromium): ${unpackedDir}`);
console.log(`   🦊 Firefox:             ${firefoxDir}`);
console.log(`   📁 ZIP:     ${zipPath}`);
console.log('\n🚀 To load the extension in Chrome:');
console.log('   1. Open chrome://extensions/');
console.log('   2. Enable "Developer mode" (top right)');
console.log('   3. Click "Load unpacked"');
console.log('   4. Select the folder:');
console.log(`      ${unpackedDir}`);
console.log(`\n   Chrome cannot load a ZIP directly: unzip ${path.basename(zipPath)} first,`);
console.log('   then pick the extracted folder in step 4.');
