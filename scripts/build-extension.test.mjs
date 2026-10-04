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

// ─── background.js 的应答关联契约 ──────────────────────────────────────────────

const backgroundSource = readFileSync(
  path.resolve(projectRoot, 'integration/extension/background.js'),
  'utf8',
);

/** 回显异常时才会出现的两条日志文案。 */
const ECHO_ANOMALY_LOGS = ['未回显 request_id', 'request_id 与请求不一致'];

/** 加载 background.js，并装一个足够跑通 Native Messaging 的 chrome 桩。 */
function loadBackground(respond) {
  const state = { posted: [], logs: '', listeners: [] };
  const port = {
    onMessage: { addListener: (fn) => (state.onMessage = fn) },
    onDisconnect: { addListener: (fn) => (state.onDisconnect = fn) },
    postMessage: (message) => {
      state.posted.push(JSON.parse(message));
      respond(state.posted[state.posted.length - 1], state);
    },
    disconnect: () => {},
  };
  globalThis.chrome = {
    runtime: {
      connectNative: () => port,
      onInstalled: { addListener: () => {} },
      onMessage: { addListener: (fn) => state.listeners.push(fn) },
      lastError: undefined,
    },
    contextMenus: { create: () => {}, onClicked: { addListener: () => {} } },
    storage: {
      local: {
        get: (key, callback) => callback({ extension_logs: state.logs }),
        set: (values) => {
          state.logs = values.extension_logs ?? state.logs;
        },
      },
    },
  };
  // background.js 是普通脚本：重新求值即可拿到一份全新的模块级状态
  new Function(backgroundSource)();
  return state;
}

test('Native Host 回显的 request_id 与请求一致时不产生异常日志', () => {
  const state = loadBackground((request, handle) => {
    handle.onMessage({
      request_id: request.request_id,
      ok: true,
      success: true,
      message: '已连接',
    });
  });

  const listener = state.listeners[0];
  const response = new Promise((resolve) => listener({ action: 'check_connection' }, {}, resolve));

  return response.then((result) => {
    assert.equal(result.success, true);
    assert.equal(result.message, '已连接');
    assert.equal(state.posted[0].version, 1);
    assert.ok(state.posted[0].request_id, '请求必须携带 request_id');
    for (const anomaly of ECHO_ANOMALY_LOGS) {
      assert.ok(!state.logs.includes(anomaly), `不应记录"${anomaly}"：${state.logs}`);
    }
  });
});

test('Host 回显的 request_id 不一致时记录日志但仍然返回应答', () => {
  const state = loadBackground((request, handle) => {
    handle.onMessage({ request_id: 'someone-elses-id', ok: true, success: true, message: '已连接' });
  });

  const listener = state.listeners[0];
  const response = new Promise((resolve) => listener({ action: 'check_connection' }, {}, resolve));

  return response.then((result) => {
    assert.equal(result.message, '已连接', '不匹配的应答仍要交给用户');
    assert.ok(
      state.logs.includes('request_id 与请求不一致'),
      `回显不一致必须留下日志：${state.logs}`,
    );
  });
});
test('旧版 Host 不回显 request_id 时按缺失处理而不是报错', () => {
  const state = loadBackground((request, handle) => {
    handle.onMessage({ success: true, message: '已连接' });
  });

  const listener = state.listeners[0];
  const response = new Promise((resolve) => listener({ action: 'check_connection' }, {}, resolve));

  return response.then((result) => {
    assert.equal(result.message, '已连接');
    assert.ok(
      state.logs.includes('未回显 request_id'),
      `缺失回显必须留下日志：${state.logs}`,
    );
  });
});

test('消息文案同时认旧的 message 与 NativeResponse 的 data.message', () => {
  const state = loadBackground((request, handle) => {
    handle.onMessage({
      request_id: request.request_id,
      ok: true,
      success: true,
      data: { message: '已加入下载' },
    });
  });

  const listener = state.listeners[0];
  const response = new Promise((resolve) => listener({ action: 'check_connection' }, {}, resolve));

  return response.then((result) => {
    assert.equal(result.message, '已加入下载');
  });
});

test('等待上限是具名常量，且超过 Native Host 的 open_app 拉起预算', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const state = loadBackground(() => {
    // 永不回应：只能靠超时收场
  });

  const listener = state.listeners[0];
  const settled = new Promise((resolve) => listener({ action: 'check_connection' }, {}, resolve));
  t.mock.timers.tick(8000);

  return settled.then((result) => {
    assert.equal(result.success, false);
    assert.equal(result.message, 'Native Host 连接超时');
  });
});

test('background.js 把等待上限与 Host 侧的耦合写在注释里', () => {
  assert.match(
    backgroundSource,
    /const NATIVE_HOST_TIMEOUT_MS = 8000;/,
    '等待上限必须是具名常量，供 native-host 的测试读取',
  );
  assert.ok(
    backgroundSource.includes('integration/native-host/src/main.rs'),
    '注释必须指明 Host 侧的文件名，改动任一侧都能看到耦合',
  );
  assert.ok(
    backgroundSource.includes('OPEN_APP_LAUNCH_BUDGET'),
    '注释必须指明 Host 侧的常量名',
  );
});

// ─── open_app / check_connection 不得把失败报成成功 ─────────────────────────

/** 让 Host 回一句固定应答，再触发一次 background 的消息处理。 */
function replyThen(action, reply) {
  const state = loadBackground((request, handle) => {
    handle.onMessage({ request_id: request.request_id, ...reply });
  });
  const listener = state.listeners[0];
  return new Promise((resolve) => listener({ action }, {}, resolve));
}

test('open_app 收到 Host 的失败应答时报失败并带出 Host 的文案', () => {
  // 拉起预算收紧到 5 秒后，Host 的失败应答会准时落进 8 秒等待窗口：
  // 此时若丢弃 success，用户会看到"已启动 Multidown"，而应用根本没起来。
  return replyThen('open_app', {
    ok: false,
    success: false,
    message: '无法启动应用，请手动启动 Multidown',
  }).then((result) => {
    assert.equal(result.success, false);
    assert.equal(result.message, '无法启动应用，请手动启动 Multidown');
  });
});

test('open_app 成功时仍然报成功', () => {
  return replyThen('open_app', { ok: true, success: true, message: '已启动 Multidown' }).then(
    (result) => {
      assert.equal(result.success, true);
      assert.equal(result.message, '已启动 Multidown');
    },
  );
});

test('open_app 只回 ok:false（没有旧 success 键）时同样判失败', () => {
  return replyThen('open_app', {
    ok: false,
    data: { message: '无法启动应用，请手动启动 Multidown' },
  }).then((result) => {
    assert.equal(result.success, false);
    assert.equal(result.message, '无法启动应用，请手动启动 Multidown');
  });
});

test('check_connection 握手失败时报失败，而不是带着失败文案显示已连接', () => {
  return replyThen('check_connection', {
    ok: false,
    success: false,
    message: 'Multidown 未运行或未就绪，请先启动 Multidown',
  }).then((result) => {
    assert.equal(result.success, false);
    assert.equal(result.message, 'Multidown 未运行或未就绪，请先启动 Multidown');
  });
});

test('check_connection 握手成功时仍然报成功', () => {
  return replyThen('check_connection', { ok: true, success: true, message: '已连接' }).then(
    (result) => {
      assert.equal(result.success, true);
      assert.equal(result.message, '已连接');
    },
  );
});

test('popup.js 在 open_app 失败时优先显示 Host 的文案', () => {
  const popupSource = readFileSync(
    path.resolve(projectRoot, 'integration/extension/popup.js'),
    'utf8',
  );
  const openAppHandler = popupSource.slice(
    popupSource.indexOf("action: 'open_app'"),
    popupSource.indexOf('loadPageMedia'),
  );
  assert.ok(
    openAppHandler.includes('response?.message'),
    'open_app 失败必须把 Host 的文案显示出来，而不是固定提示',
  );
});
