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
      }, 8000);

      port.onMessage.addListener(response => {
        clearTimeout(timeout);
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

      // 协议 v1：请求携带版本与请求 ID，Host 会原样回显请求 ID
      const request_id = typeof crypto !== 'undefined' && crypto.randomUUID
        ? crypto.randomUUID()
        : `req-${Date.now()}-${Math.random().toString(36).slice(2)}`;
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
          return { success: true, message: resp?.message || '已连接' };
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
        return { success: result.success !== false, message: result?.message || '已添加' };

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
        return { success: result.success !== false, message: result?.message || '已添加' };

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
        // Try to open app via native messaging ping
        try {
          await sendToNativeHost('open_app', {});
          return { success: true };
        } catch (e) {
          return { success: false, message: '无法启动应用' };
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