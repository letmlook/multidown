// build-extension.mjs - Build and package the Multidown Chrome Extension
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { execSync } from 'child_process';
import { createWriteStream } from 'fs';
import { pipeline } from 'stream/promises';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');
const extensionDir = path.resolve(projectRoot, 'integration', 'extension');
const outputDir = path.resolve(projectRoot, 'dist-extension');

// Ensure output directory exists
if (!fs.existsSync(outputDir)) {
  fs.mkdirSync(outputDir, { recursive: true });
}

// Copy extension files to dist folder (unpacked)
const unpackedDir = path.resolve(outputDir, 'unpacked');
if (fs.existsSync(unpackedDir)) {
  fs.rmSync(unpackedDir, { recursive: true });
}
fs.mkdirSync(unpackedDir, { recursive: true });

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

console.log('📦 Copying extension files to unpacked directory...');
copyDir(extensionDir, unpackedDir);

// Also copy icons if they exist separately
const srcIcons = path.join(extensionDir, 'icons');
const destIcons = path.join(unpackedDir, 'icons');
if (fs.existsSync(srcIcons) && !fs.existsSync(destIcons)) {
  copyDir(srcIcons, destIcons);
}
console.log(`   Copied to: ${unpackedDir}`);

// Generate a simple ZIP package for Chrome developer mode loading
const zipPath = path.resolve(outputDir, 'multidown-extension.zip');
console.log('\n📁 Creating ZIP package...');

try {
  // Use python3 to create zip (available on all platforms)
  execSync(`cd "${unpackedDir}" && python3 -c "
import zipfile, os, sys
zip_path = '${zipPath.replace(/\\\\/g, '\\\\\\\\')}'
with zipfile.ZipFile(zip_path, 'w', zipfile.ZIP_DEFLATED) as zf:
    for root, dirs, files in os.walk('.'):
        for file in files:
            filepath = os.path.join(root, file)
            arcname = filepath[2:] if filepath.startswith('./') else filepath
            zf.write(filepath, arcname)
print('ZIP created:', zip_path)
"`, { stdio: 'inherit' });
} catch (e) {
  // Fallback: just keep the unpacked directory
  console.log('⚠️  ZIP creation skipped, use unpacked directory in Chrome');
}

// Generate icons if they don't exist
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
  try {
    createIcon(size);
  } catch (e) {
    console.log(`   Skipped icon ${size} (not critical)`);
  }
});

console.log('\n==========================================');
console.log('   Extension Build Complete!');
console.log('==========================================');
console.log('\n📦 Extension packages:');
console.log(`   📂 Unpacked: ${unpackedDir}`);
if (fs.existsSync(zipPath)) {
  console.log(`   📁 ZIP:     ${zipPath}`);
}
console.log('\n🚀 To load the extension in Chrome:');
console.log('   1. Open chrome://extensions/');
console.log('   2. Enable "Developer mode" (top right)');
console.log('   3. Click "Load unpacked"');
console.log('   4. Select the folder:');
console.log(`      ${unpackedDir}`);