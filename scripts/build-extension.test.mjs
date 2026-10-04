// build-extension.test.mjs - Extension packaging contract tests (plain Node, no test framework).
//
// Run with: node scripts/build-extension.test.mjs
//
// The suite inspects the real ZIP bytes produced by build-extension.mjs with an
// independent reader, so a malformed archive or a flattened/prefixed nested path
// fails here instead of surfacing in a browser.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

import { collectExtensionFiles, createZip } from './build-extension.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');
const outputDir = path.resolve(projectRoot, 'dist-extension');
const zipPath = path.resolve(outputDir, 'multidown-extension.zip');
const unpackedDir = path.resolve(outputDir, 'unpacked');
const firefoxDir = path.resolve(outputDir, 'firefox-unpacked');

const EOCD_SIGNATURE = 0x06054b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const LOCAL_SIGNATURE = 0x04034b50;

/** Parse a ZIP buffer into `{ name, method, data }` entries via the central directory. */
function readZipEntries(buffer) {
  let eocd = -1;
  for (let offset = buffer.length - 22; offset >= 0; offset -= 1) {
    if (buffer.readUInt32LE(offset) === EOCD_SIGNATURE) {
      eocd = offset;
      break;
    }
  }
  assert.notEqual(eocd, -1, 'ZIP 缺少中央目录结束记录');

  const entryCount = buffer.readUInt16LE(eocd + 10);
  let cursor = buffer.readUInt32LE(eocd + 16);
  const entries = [];

  for (let index = 0; index < entryCount; index += 1) {
    assert.equal(buffer.readUInt32LE(cursor), CENTRAL_SIGNATURE, '中央目录条目签名不匹配');
    const method = buffer.readUInt16LE(cursor + 10);
    const compressedSize = buffer.readUInt32LE(cursor + 20);
    const nameLength = buffer.readUInt16LE(cursor + 28);
    const extraLength = buffer.readUInt16LE(cursor + 30);
    const commentLength = buffer.readUInt16LE(cursor + 32);
    const localOffset = buffer.readUInt32LE(cursor + 42);
    const name = buffer.toString('utf8', cursor + 46, cursor + 46 + nameLength);

    assert.equal(
      buffer.readUInt32LE(localOffset),
      LOCAL_SIGNATURE,
      `本地文件头签名不匹配: ${name}`,
    );
    const localNameLength = buffer.readUInt16LE(localOffset + 26);
    const localExtraLength = buffer.readUInt16LE(localOffset + 28);
    const dataStart = localOffset + 30 + localNameLength + localExtraLength;
    const raw = buffer.subarray(dataStart, dataStart + compressedSize);
    const data = method === 0 ? Buffer.from(raw) : zlib.inflateRawSync(raw);

    entries.push({ name, method, data });
    cursor += 46 + nameLength + extraLength + commentLength;
  }

  return entries;
}

function entryNames(buffer) {
  return readZipEntries(buffer)
    .map((entry) => entry.name)
    .sort();
}

function writeFixtureTree(root) {
  const files = {
    'manifest.json': '{"manifest_version":3,"name":"fixture"}\n',
    'background.js': '// background\n',
    'icons/icon16.png': 'PNG-16',
    'icons/icon48.png': 'PNG-48',
  };
  for (const [relativePath, contents] of Object.entries(files)) {
    const absolutePath = path.join(root, relativePath);
    mkdirSync(path.dirname(absolutePath), { recursive: true });
    writeFileSync(absolutePath, contents);
  }
  return files;
}

test('collectExtensionFiles 返回以 / 分隔的嵌套相对路径', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'multidown-zip-entries-'));
  try {
    const source = path.join(root, 'source');
    mkdirSync(source, { recursive: true });
    const files = writeFixtureTree(source);

    const collected = collectExtensionFiles(source);

    assert.deepEqual(
      collected.map((entry) => entry.archivePath),
      ['background.js', 'icons/icon16.png', 'icons/icon48.png', 'manifest.json'],
    );
    for (const [relativePath, contents] of Object.entries(files)) {
      const match = collected.find((entry) => entry.archivePath === relativePath);
      assert.equal(readFileSync(match.absolutePath, 'utf8'), contents);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('createZip 保留嵌套相对路径且内容可还原（无 CRX 参与）', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'multidown-zip-roundtrip-'));
  try {
    const source = path.join(root, 'source');
    mkdirSync(source, { recursive: true });
    const files = writeFixtureTree(source);
    const archive = path.join(root, 'fixture.zip');

    createZip(collectExtensionFiles(source), archive);

    const entries = readZipEntries(readFileSync(archive));
    assert.deepEqual(
      entries.map((entry) => entry.name).sort(),
      ['background.js', 'icons/icon16.png', 'icons/icon48.png', 'manifest.json'],
    );
    for (const [relativePath, contents] of Object.entries(files)) {
      const entry = entries.find((candidate) => candidate.name === relativePath);
      assert.ok(entry, `ZIP 缺少条目 ${relativePath}`);
      assert.equal(entry.data.toString('utf8'), contents, `条目内容不一致: ${relativePath}`);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('构建产物 ZIP 保留 icons/ 等嵌套路径', () => {
  execFileSync(process.execPath, [path.join(__dirname, 'build-extension.mjs')], {
    cwd: projectRoot,
    stdio: 'pipe',
  });

  assert.ok(existsSync(zipPath), `缺少 ZIP 产物: ${zipPath}`);
  const names = entryNames(readFileSync(zipPath));

  for (const expected of [
    'background.js',
    'com.multidown.app.json',
    'content.css',
    'content.js',
    'icons/icon128.png',
    'icons/icon16.png',
    'icons/icon48.png',
    'manifest.json',
    'popup.html',
    'popup.js',
  ]) {
    assert.ok(names.includes(expected), `ZIP 缺少 ${expected}，实际条目：${names.join(', ')}`);
  }
  for (const name of names) {
    assert.ok(!name.startsWith('unpacked/'), `ZIP 条目不应带目录前缀: ${name}`);
    assert.ok(!name.includes('\\'), `ZIP 条目必须使用 / 分隔: ${name}`);
    assert.ok(!name.split('/').includes('..'), `ZIP 条目不得逃逸根目录: ${name}`);
  }

  const entries = readZipEntries(readFileSync(zipPath));
  const icon = entries.find((entry) => entry.name === 'icons/icon16.png');
  assert.equal(
    icon.data.toString('base64'),
    readFileSync(path.join(outputDir, 'unpacked', 'icons', 'icon16.png')).toString('base64'),
    'ZIP 中的 icons/icon16.png 与解压目录内容不一致',
  );
});

test('ZIP 内的 manifest 保留稳定扩展 ID，且不再产出 CRX', () => {
  const source = JSON.parse(
    readFileSync(path.resolve(projectRoot, 'integration/extension/manifest.json'), 'utf8'),
  );
  const entries = readZipEntries(readFileSync(zipPath));
  const manifest = JSON.parse(
    entries.find((entry) => entry.name === 'manifest.json').data.toString('utf8'),
  );

  assert.equal(manifest.key, source.key, 'ZIP 打包不得改变 Chromium 稳定扩展 ID');
  assert.ok(
    !existsSync(path.join(outputDir, 'multidown-extension.crx')),
    '构建不应产出 CRX 产物',
  );

  const { scripts } = JSON.parse(readFileSync(path.resolve(projectRoot, 'package.json'), 'utf8'));
  assert.ok(!scripts['build:all'].includes('sign:crx'), 'build:all 不得依赖 CRX 签名');
});

test('Firefox 变体仍保留 gecko ID 且不携带 Chromium key', () => {
  const manifest = JSON.parse(readFileSync(path.join(firefoxDir, 'manifest.json'), 'utf8'));

  assert.equal(manifest.browser_specific_settings.gecko.id, 'multidown@letmlook');
  assert.equal(manifest.key, undefined);
  assert.deepEqual(manifest.background, { scripts: ['background.js'] });
});
