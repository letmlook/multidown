const HOST_NAME = "com.multidown.app";

// 简化版 background.js，确保 Service Worker 能够正常注册

// 调试日志函数
function debugLog(message, data = {}) {
  // 输出到控制台
  console.log(`[Multidown Debug] ${message}`, data);
  
  // 尝试写入本地日志文件
  writeToLogFile(message, data);
}

// 写入日志到本地文件
function writeToLogFile(message, data = {}) {
  try {
    // 构建日志消息
    const timestamp = new Date().toISOString();
    const dataString = Object.keys(data).length > 0 ? JSON.stringify(data) : '';
    const logMessage = `[${timestamp}] [Multidown Extension] ${message} ${dataString}\n`;
    
    // 使用 chrome.storage.local 存储日志
    chrome.storage.local.get('extension_logs', function(result) {
      let logs = result.extension_logs || '';
      
      // 限制日志大小，防止存储空间不足
      const maxLogSize = 1024 * 1024; // 1MB
      if (logs.length > maxLogSize) {
        // 截断日志，保留最新的部分
        logs = logs.substring(logs.length - maxLogSize);
      }
      
      // 添加新日志
      logs += logMessage;
      
      // 保存回存储
      chrome.storage.local.set({ 'extension_logs': logs }, function() {
        // 存储成功，无需操作
      });
    });
  } catch (e) {
    // 存储失败时不影响其他功能
    console.warn('无法写入本地日志文件:', e);
  }
}

// 导出日志函数（可选，用于调试）
function exportLogs() {
  chrome.storage.local.get('extension_logs', function(result) {
    const logs = result.extension_logs || '';
    const blob = new Blob([logs], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'multidown-extension-logs.txt';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  });
}

// 确保 chrome 全局对象存在
if (typeof chrome !== 'undefined') {
  // 扩展安装时创建上下文菜单
  if (chrome.runtime && chrome.runtime.onInstalled) {
    chrome.runtime.onInstalled.addListener(() => {
      debugLog("扩展已安装，创建上下文菜单");
      
      if (chrome.contextMenus) {
        // 创建链接下载菜单
        chrome.contextMenus.create({
          id: "multidown-link",
          title: "使用 Multidown 下载链接",
          contexts: ["link"],
        });
        
        // 创建页面下载菜单
        chrome.contextMenus.create({
          id: "multidown-page",
          title: "使用 Multidown 下载此页面",
          contexts: ["page"],
        });
        
        // 创建视频下载菜单
        chrome.contextMenus.create({
          id: "multidown-video",
          title: "使用 Multidown 下载视频",
          contexts: ["video"],
        });
        
        // 创建音频下载菜单
        chrome.contextMenus.create({
          id: "multidown-audio",
          title: "使用 Multidown 下载音频",
          contexts: ["audio"],
        });
        
        debugLog("上下文菜单创建完成");
      } else {
        debugLog("chrome.contextMenus 不可用");
      }
    });
  }
  
  // 上下文菜单项点击事件
  if (chrome.contextMenus && chrome.contextMenus.onClicked) {
    chrome.contextMenus.onClicked.addListener((info, tab) => {
      debugLog("上下文菜单项被点击", { menuItemId: info.menuItemId, tabId: tab?.id });
      
      let url = "";
      let filename = "";
      
      if (info.menuItemId === "multidown-link" && info.linkUrl) {
        url = info.linkUrl;
        filename = info.linkText || info.linkUrl.split('/').pop();
        debugLog("处理链接下载", { url, filename, linkText: info.linkText });
      } else if (info.menuItemId === "multidown-page" && tab?.url) {
        url = tab.url;
        filename = tab.title || tab.url.split('/').pop();
        debugLog("处理页面下载", { url, filename, tabTitle: tab.title });
      } else if (info.menuItemId === "multidown-video" && info.srcUrl) {
        url = info.srcUrl;
        filename = "video_" + Date.now() + ".mp4";
        debugLog("处理视频下载", { url, filename });
      } else if (info.menuItemId === "multidown-audio" && info.srcUrl) {
        url = info.srcUrl;
        filename = "audio_" + Date.now() + ".mp3";
        debugLog("处理音频下载", { url, filename });
      }
      
      if (!url || (!url.startsWith("http://") && !url.startsWith("https://"))) {
        debugLog("无效的URL，跳过下载", { url });
        return;
      }
      
      // 与IDM对齐的命令结构
      const downloadData = {
        url: url,
        filename: filename,
        referer: tab?.url || "",
        user_agent: info.userAgent || "",
        cookie: "",
        post_data: "",
        save_path: "",
        open_window: true
      };
      
      debugLog("准备发送下载请求", downloadData);
      sendToNativeHost("download", downloadData);
    });
  }
  
  // 扩展启动事件
  if (chrome.runtime && chrome.runtime.onStartup) {
    chrome.runtime.onStartup.addListener(() => {
      debugLog("扩展启动");
    });
  }
  
  // 扩展错误事件
  if (chrome.runtime && chrome.runtime.onError) {
    chrome.runtime.onError.addListener((error) => {
      debugLog("扩展错误", { error });
    });
  }
} else {
  console.error("Chrome 扩展 API 不可用");
}

// 发送消息到本地主机
function sendToNativeHost(action, data) {
  debugLog("发送消息到本地主机", { action, data });
  
  try {
    if (chrome.runtime && chrome.runtime.connectNative) {
      debugLog("尝试连接本地主机", { hostName: HOST_NAME });
      const port = chrome.runtime.connectNative(HOST_NAME);
      
      debugLog("连接成功，设置消息监听器");
      
      if (port && port.onMessage) {
        port.onMessage.addListener((response) => {
          debugLog("收到本地主机响应", { response });
          if (response && response.success) {
            console.log("Multidown:", response.message);
          } else {
            console.warn("Multidown:", response?.message || "未知错误");
          }
        });
      }
      
      if (port && port.onDisconnect) {
        port.onDisconnect.addListener(() => {
          if (chrome.runtime.lastError) {
            debugLog("Native Host 连接断开", { error: chrome.runtime.lastError });
            console.warn("Multidown 扩展: Native Host 连接断开。", chrome.runtime.lastError);
          } else {
            debugLog("Native Host 连接正常关闭");
          }
        });
      }
      
      if (port && port.postMessage) {
        const message = { action, ...data };
        debugLog("发送消息数据", { message });
        port.postMessage(message);
        
        debugLog("消息发送完成");
      }
    } else {
      debugLog("Chrome runtime API 不可用");
    }
  } catch (e) {
    debugLog("连接 Native Host 失败", { error: e });
    console.warn("Multidown 扩展: 无法连接 Native Host。请确保已安装 Multidown 并已注册 Native Messaging Host。", e);
  }
}
