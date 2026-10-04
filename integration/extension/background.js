// background.js - Multidown Extension Background Service Worker
// Communication: Extension <-> Native Host <-> Main App (TCP)

const HOST_NAME = 'com.multidown.app';

// ─── Debug Logging ────────────────────────────────────────────────────────────

function debugLog(message, data = {}) {
  const timestamp = new Date().toISOString();
  const dataStr = Object.keys(data).length ? JSON.stringify(data) : '';
  const logLine = `[${timestamp}] [Multidown] ${message} ${dataStr}\n`;
  chrome.storage.local.get('extension_logs', result => {
    let logs = result.extension_logs || '';
    const MAX = 500 * 1024;
    if (logs.length > MAX) logs = logs.slice(-MAX);
    chrome.storage.local.set({ 'extension_logs': logs + logLine });
  });
  console.log(`[Multidown] ${message}`, data);
}

// ─── Native Messaging ─────────────────────────────────────────────────────────

// 等待 Native Host 应答的上限（毫秒）。
// 与 `integration/native-host/src/main.rs` 的 `OPEN_APP_LAUNCH_BUDGET` 绑定：
// Host 端 open_app 的拉起预算是 5 秒，最坏耗时是
// 预算 5s + 握手 IO 1s（写）+ 1s（读）+ 400ms 轮询 ≈ 7.4s，
// 必须严格小于这里的上限，否则深链拉起失败时用户只会看到"Native Host 连接超时"，
// 而 Host 写好的"无法启动应用，请手动启动 Multidown"会晚到已被放弃的管道里。
// 余量只有 0.6s：这个数字被低估过（曾按"握手 IO 只有 1s"算），而低估的余量
// 会悄悄消失。改动任一侧都必须同步另一侧；native-host 里有测试把这两个数字
// 和上面的算式绑在一起。
const NATIVE_HOST_TIMEOUT_MS = 8000;

// 应答消息：Host 同时给出旧的 `message` 与 NativeResponse 的 `data.message`，
// 两个位置都认，新旧 Host 都能显示可读文案。
function replyMessage(response, fallback) {
  if (!response) return fallback;
  return response.message || response.data?.message || fallback;
}

// 应答是否表示失败。`success` 是旧 Host 的扁平键，`ok` 是 NativeResponse 的
// 信封键，两个都看：Host 的失败应答现在会在 8 秒等待之内准时到达（拉起预算
// 从 15 秒收紧到 5 秒），只把 `success` 当成功会把失败报成成功。
// 两个键都缺时按成功处理——那与"旧 Host 不回显"时的既有行为一致。
function replyFailed(response) {
  return !!response && (response.success === false || response.ok === false);
}

// Send a message to the Native Host via Chrome Native Messaging
// Protocol: stdin writes 4-byte LE length + JSON; stdout reads 4-byte LE + JSON response
function sendToNativeHost(action, data, useNative = true) {
  return new Promise((resolve, reject) => {
    if (!useNative) {
      // Fallback: just log and return success for testing
      resolve({ success: true, message: '测试模式：消息已记录', action });
      return;
    }
    try {
      const port = chrome.runtime.connectNative(HOST_NAME);
      const timeout = setTimeout(() => {
        port.disconnect();
        reject(new Error('Native Host 连接超时'));
      }, NATIVE_HOST_TIMEOUT_MS);

      // 协议 v1：请求携带版本与请求 ID。Host 在应答里回显 request_id；
      // 旧版 Host 不回显，此时按"缺失"记录日志并照常使用应答。
      const request_id = typeof crypto !== 'undefined' && crypto.randomUUID
        ? crypto.randomUUID()
        : `req-${Date.now()}-${Math.random().toString(36).slice(2)}`;

      port.onMessage.addListener(response => {
        clearTimeout(timeout);
        const echoed = response && response.request_id;
        if (!echoed) {
          debugLog('Native Host 应答未回显 request_id', { action, response });
        } else if (echoed !== request_id) {
          debugLog('Native Host 回显的 request_id 与请求不一致', { action });
        }
        debugLog('Native Host 响应', response);
        resolve(response);
        port.disconnect();
      });
      port.onDisconnect.addListener(() => {
        clearTimeout(timeout);
        if (chrome.runtime.lastError) {
          debugLog('Native Host 断开错误', { error: chrome.runtime.lastError.message });
          reject(new Error(chrome.runtime.lastError.message));
        }
      });

      const message = JSON.stringify({ version: 1, request_id, action, ...data });
      debugLog('发送 Native Host 消息', { request_id, action, data });
      port.postMessage(message);
    } catch (e) {
      debugLog('Native Host 连接异常', { error: e.message });
      reject(e);
    }
  });
}

// ─── Capture Rules（与主程序设置同步：总开关 + 域名黑名单） ─────────────────────

let captureConfig = null;
let captureConfigFetchedAt = 0;
const CAPTURE_CONFIG_TTL_MS = 60 * 1000;

function hostOf(url) {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return '';
  }
}

async function getCaptureConfig(force = false) {
  const now = Date.now();
  if (!force && captureConfig && now - captureConfigFetchedAt < CAPTURE_CONFIG_TTL_MS) {
    return captureConfig;
  }
  try {
    const resp = await sendToNativeHost('get_config', {});
    if (resp && resp.success && resp.config) {
      captureConfig = {
        capture_enabled: resp.config.capture_enabled !== false,
        domain_blacklist: Array.isArray(resp.config.domain_blacklist)
          ? resp.config.domain_blacklist
          : [],
      };
      captureConfigFetchedAt = now;
    }
  } catch {
    // 主程序未运行时不拦截，行为与旧版一致
  }
  return captureConfig || { capture_enabled: true, domain_blacklist: [] };
}

// 返回 null 表示放行；否则返回拒绝原因字符串
async function checkCaptureRules(url) {
  const cfg = await getCaptureConfig();
  if (!cfg.capture_enabled) {
    return '浏览器捕获已在 Multidown 中关闭';
  }
  const host = hostOf(url);
  const blocked = cfg.domain_blacklist.some((entry) => {
    const e = String(entry).trim().toLowerCase();
    return e && (host === e || host.endsWith('.' + e));
  });
  if (blocked) {
    return '该域名已被捕获黑名单过滤';
  }
  return null;
}

// ─── Context Menus ────────────────────────────────────────────────────────────

chrome.runtime.onInstalled.addListener(() => {
  debugLog('扩展已安装，创建上下文菜单');

  chrome.contextMenus.create({
    id: 'multidown-link',
    title: '使用 Multidown 下载链接',
    contexts: ['link']
  });
  chrome.contextMenus.create({
    id: 'multidown-magnet',
    title: '使用 Multidown 下载磁力链接',
    contexts: ['link'],
    // 仅磁力链接显示该项（普通链接用上面的通用项）
    targetUrlPatterns: ['magnet:*']
  });
  chrome.contextMenus.create({
    id: 'multidown-page',
    title: '使用 Multidown 下载此页面',
    contexts: ['page']
  });
  chrome.contextMenus.create({
    id: 'multidown-video',
    title: '使用 Multidown 下载视频',
    contexts: ['video']
  });
  chrome.contextMenus.create({
    id: 'multidown-audio',
    title: '使用 Multidown 下载音频',
    contexts: ['audio']
  });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  let url = '', filename = '';
  if (info.menuItemId === 'multidown-magnet' && info.linkUrl) {
    // 磁力链接：显示名取 dn 参数（若有）
    url = info.linkUrl;
    const dn = new URLSearchParams(url.replace(/^magnet:\?/, '')).get('dn');
    filename = dn || 'magnet-download';
  } else if (info.menuItemId === 'multidown-link' && info.linkUrl) {
    url = info.linkUrl;
    filename = info.linkUrl.split('/').pop() || 'download';
  } else if (info.menuItemId === 'multidown-page' && tab?.url) {
    url = tab.url;
    filename = tab.title || 'page';
  } else if (info.menuItemId === 'multidown-video' && info.srcUrl) {
    url = info.srcUrl;
    filename = 'video_' + Date.now() + '.mp4';
  } else if (info.menuItemId === 'multidown-audio' && info.srcUrl) {
    url = info.srcUrl;
    filename = 'audio_' + Date.now() + '.mp3';
  }

  const isTorrentUrl =
    url.startsWith('magnet:') || url.toLowerCase().endsWith('.torrent');
  if (!url || (!url.startsWith('http://') && !url.startsWith('https://') && !isTorrentUrl)) {
    debugLog('无效URL，跳过', { url });
    return;
  }

  const downloadData = {
    url,
    filename,
    referer: tab?.url || '',
    user_agent: info.userAgent || '',
    cookie: '',
    post_data: '',
    save_path: '',
    open_window: true
  };

  debugLog('处理上下文菜单下载', { menuItemId: info.menuItemId, url, filename });
  checkCaptureRules(url).then((rejected) => {
    if (rejected) {
      debugLog('捕获规则拒绝下载', { url, reason: rejected });
      chrome.notifications
        ?.create({
          type: 'basic',
          iconUrl: 'icons/icon128.png',
          title: 'Multidown',
          message: rejected,
        })
        .catch?.(() => {});
      return;
    }
    sendToNativeHost('download', downloadData).catch(e => {
      console.warn('[Multidown] 下载失败:', e.message);
    });
  });
});

// ─── Message Handling ─────────────────────────────────────────────────────────

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  debugLog('收到消息', { action: message.action, from: sender.tab?.url || 'popup' });

  const handleAsync = async () => {
    try {
      if (message.action === 'check_connection') {
        // Quick connectivity check
        try {
          const resp = await sendToNativeHost('test_connection', {});
          // 握手失败时 Host 回的是 {success:false,…}，文案是
          // "Multidown 未运行或未就绪"。丢掉 success 把它当成功返回，
          // 弹窗就会一边报错文案一边显示"已连接"。
          if (replyFailed(resp)) {
            return { success: false, message: replyMessage(resp, 'Multidown 未运行或未就绪') };
          }
          return { success: true, message: replyMessage(resp, '已连接') };
        } catch (e) {
          return { success: false, message: e.message };
        }

      } else if (message.action === 'quick_download') {
        const rejected = await checkCaptureRules(message.url);
        if (rejected) {
          return { success: false, message: rejected };
        }
        // Direct URL download from popup
        const result = await sendToNativeHost('download', {
          url: message.url,
          filename: message.filename || message.url.split('/').pop() || 'download',
          referer: '',
          user_agent: '',
          cookie: '',
          post_data: '',
          save_path: '',
          open_window: true
        });
        return { success: result.success !== false, message: replyMessage(result, '已添加') };

      } else if (message.action === 'download_media') {
        const rejected = await checkCaptureRules(message.url);
        if (rejected) {
          return { success: false, message: rejected };
        }
        // Media download from content script
        const result = await sendToNativeHost('download', {
          url: message.url,
          filename: message.filename || 'download',
          referer: sender.tab?.url || '',
          user_agent: navigator.userAgent,
          cookie: '',
          post_data: '',
          save_path: '',
          open_window: true
        });
        return { success: result.success !== false, message: replyMessage(result, '已添加') };

      } else if (message.action === 'export_logs') {
        const result = await new Promise(resolve => {
          chrome.storage.local.get('extension_logs', r => resolve(r.extension_logs || ''));
        });
        const blob = new Blob([result], { type: 'text/plain' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = 'multidown-extension-logs.txt';
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
        return { success: true };

      } else if (message.action === 'open_app') {
        // 通过 native messaging 拉起主程序
        try {
          const resp = await sendToNativeHost('open_app', {});
          // 必须看 Host 的成功标志：拉起预算是 5 秒，它的失败应答会在 8 秒
          // 等待之内准时到达（更早的版本预算是 15 秒，应答总是晚到、由超时
          // 兜底）。无视 success 会把"应用根本没起来"报成"已启动 Multidown"。
          if (replyFailed(resp)) {
            return {
              success: false,
              message: replyMessage(resp, '无法启动应用，请手动启动 Multidown'),
            };
          }
          return { success: true, message: replyMessage(resp, '已启动 Multidown') };
        } catch (e) {
          return { success: false, message: e.message };
        }

      } else {
        return { success: false, message: '未知命令: ' + message.action };
      }
    } catch (e) {
      debugLog('消息处理异常', { action: message.action, error: e.message });
      return { success: false, message: e.message };
    }
  };

  handleAsync().then(sendResponse);
  return true; // async response
});