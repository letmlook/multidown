// popup.js - Multidown Extension Popup Script

// Toast notification
function showToast(message, type = 'info') {
  const toast = document.getElementById('toast');
  toast.textContent = message;
  toast.className = `toast ${type} show`;
  setTimeout(() => { toast.className = 'toast'; }, 3000);
}

// Update status indicator
function updateStatus(status, message) {
  const dot = document.getElementById('statusDot');
  const text = document.getElementById('statusText');
  dot.className = 'status-dot ' + (status === 'ready' ? 'ready' : status === 'error' ? 'error' : '');
  text.className = 'status-text ' + (status === 'ready' ? 'ready' : status === 'error' ? 'error' : '');
  text.textContent = message;
}

// Quick download from URL input
document.getElementById('btnDownload').addEventListener('click', function() {
  const url = document.getElementById('urlInput').value.trim();
  if (!url) {
    showToast('请输入下载链接', 'error');
    return;
  }
  if (!url.startsWith('http://') && !url.startsWith('https://')) {
    showToast('链接格式无效', 'error');
    return;
  }
  document.getElementById('btnDownload').disabled = true;
  chrome.runtime.sendMessage({
    action: 'quick_download',
    url: url,
    filename: url.split('/').pop() || 'download'
  }, function(response) {
    document.getElementById('btnDownload').disabled = false;
    if (response && response.success) {
      showToast('已添加到下载队列', 'success');
      document.getElementById('urlInput').value = '';
    } else {
      showToast(response?.message || '下载失败', 'error');
    }
  });
});

// Enter key on URL input
document.getElementById('urlInput').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') {
    document.getElementById('btnDownload').click();
  }
});

// Export logs
document.getElementById('btnLogs').addEventListener('click', function() {
  chrome.runtime.sendMessage({ action: 'export_logs' }, function(response) {
    if (response && response.success) {
      showToast('日志已导出', 'success');
    } else {
      showToast('导出失败', 'error');
    }
  });
});

// Open main app
document.getElementById('btnOpenApp').addEventListener('click', function() {
  chrome.runtime.sendMessage({ action: 'open_app' }, function(response) {
    if (response && response.success) {
      showToast('已启动 Multidown', 'success');
    } else {
      showToast('启动失败，请确保已安装', 'error');
    }
  });
});

// Load detected media from content script
function loadPageMedia() {
  chrome.tabs.query({ active: true, currentWindow: true }, function(tabs) {
    if (!tabs[0]) return;
    chrome.tabs.sendMessage(tabs[0].id, { action: 'get_media' }, function(response) {
      const section = document.getElementById('mediaSection');
      const noMedia = document.getElementById('noMedia');
      if (!response || !response.media || response.media.length === 0) {
        noMedia.innerHTML = '<div class="icon">🎬</div><div>当前页面未检测到媒体</div>';
        return;
      }
      noMedia.style.display = 'none';
      section.innerHTML = response.media.slice(0, 5).map(function(m) {
        return '<div class="media-item">' +
          '<div class="media-icon ' + m.type + '">' + (m.type === 'video' ? '🎬' : m.type === 'audio' ? '🎵' : '🖼') + '</div>' +
          '<div class="media-info">' +
            '<div class="media-name">' + (m.name || m.url.split('/').pop() || '未命名') + '</div>' +
            '<div class="media-type">' + m.type + '</div>' +
          '</div>' +
          '<button class="btn-media-download" data-url="' + encodeURIComponent(m.url) + '" data-type="' + m.type + '">下载</button>' +
        '</div>';
      }).join('');

      // Attach download handlers
      section.querySelectorAll('.btn-media-download').forEach(function(btn) {
        btn.addEventListener('click', function() {
          const url = decodeURIComponent(this.dataset.url);
          const type = this.dataset.type;
          chrome.runtime.sendMessage({
            action: 'download_media',
            mediaType: type,
            url: url,
            filename: type + '_' + Date.now() + (type === 'video' ? '.mp4' : type === 'audio' ? '.mp3' : '.jpg')
          }, function(response) {
            if (response && response.success) {
              showToast('已添加到下载队列', 'success');
            } else {
              showToast(response?.message || '下载失败', 'error');
            }
          });
        });
      });
    });
  });
}

// Initialize
function init() {
  // Check connection
  chrome.runtime.sendMessage({ action: 'check_connection' }, function(response) {
    if (response && response.success) {
      updateStatus('ready', '已连接 Multidown');
    } else {
      updateStatus('error', '未检测到 Multidown');
    }
  });

  // Load page media
  loadPageMedia();
}

init();
