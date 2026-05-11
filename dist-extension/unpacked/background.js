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

      const message = JSON.stringify({ action, ...data });
      debugLog('发送 Native Host 消息', { action, data: data });
      port.postMessage(message);
    } catch (e) {
      debugLog('Native Host 连接异常', { error: e.message });
      reject(e);
    }
  });
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
  if (info.menuItemId === 'multidown-link' && info.linkUrl) {
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

  if (!url || (!url.startsWith('http://') && !url.startsWith('https://'))) {
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
  sendToNativeHost('download', downloadData).catch(e => {
    console.warn('[Multidown] 下载失败:', e.message);
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