// content.js - Multidown Extension Content Script
// Detects media on the page and injects download buttons

(function() {
  'use strict';

  const BUTTON_ID = 'multidown-fab-btn';
  const MEDIA_TYPES = { VIDEO: 'video', AUDIO: 'audio', IMAGE: 'image' };

  // Track injected buttons to avoid duplicates
  const trackedMedia = new WeakSet();

  // Get media info from a video/audio element
  function getMediaInfo(el, type) {
    const url = el.src || (el.currentSrc && el.currentSrc()) || '';
    if (!url || !url.startsWith('http')) return null;

    let name = '';
    try {
      const urlObj = new URL(url);
      name = urlObj.pathname.split('/').pop() || '';
      if (!name || name.length > 100) name = type + '_' + Date.now();
    } catch (e) {
      name = type + '_' + Date.now();
    }

    return {
      type: type,
      url: url,
      name: name
    };
  }

  // Get image info
  function getImageInfo(img) {
    // Only include images that are large enough to be downloadable
    if (img.naturalWidth < 200 || img.naturalHeight < 200) return null;
    const url = img.src || '';
    if (!url || !url.startsWith('http')) return null;

    // Skip tiny images, icons, tracking pixels
    if (img.naturalWidth < 200 || img.naturalHeight < 200) return null;

    let name = '';
    try {
      const urlObj = new URL(url);
      name = urlObj.pathname.split('/').pop() || '';
      if (!name || name.length > 100 || !name.includes('.')) name = 'image_' + Date.now() + '.jpg';
    } catch (e) {
      name = 'image_' + Date.now() + '.jpg';
    }

    return {
      type: 'image',
      url: url,
      name: name
    };
  }

  // Create a floating download button for a media element
  function createDownloadButton(mediaInfo) {
    if (trackedMedia.has(mediaInfo.el)) return;
    trackedMedia.add(mediaInfo.el);

    const btn = document.createElement('div');
    btn.className = 'multidown-download-fab';
    btn.title = '使用 Multidown 下载';
    btn.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>';
    btn.style.cssText = [
      'position: absolute',
      'z-index: 2147483647',
      'width: 36px',
      'height: 36px',
      'border-radius: 50%',
      'background: linear-gradient(135deg, #00f5ff, #0080ff)',
      'border: none',
      'cursor: pointer',
      'display: flex',
      'align-items: center',
      'justify-content: center',
      'box-shadow: 0 4px 16px rgba(0, 245, 255, 0.4)',
      'transition: transform 0.2s, box-shadow 0.2s',
      'color: #000',
      'pointer-events: auto'
    ].join(';');

    btn.addEventListener('mouseenter', function() {
      btn.style.transform = 'scale(1.15)';
      btn.style.boxShadow = '0 6px 24px rgba(0, 245, 255, 0.6)';
    });
    btn.addEventListener('mouseleave', function() {
      btn.style.transform = 'scale(1)';
      btn.style.boxShadow = '0 4px 16px rgba(0, 245, 255, 0.4)';
    });
    btn.addEventListener('click', function(e) {
      e.preventDefault();
      e.stopPropagation();
      chrome.runtime.sendMessage({
        action: 'download_media',
        mediaType: mediaInfo.type,
        url: mediaInfo.url,
        filename: mediaInfo.name
      }, function(response) {
        if (response && response.success) {
          showFABFeedback(btn, 'success');
        } else {
          showFABFeedback(btn, 'error');
        }
      });
    });

    // Position the button
    positionButton(btn, mediaInfo.el);

    // Observe for size/position changes
    if (typeof ResizeObserver !== 'undefined') {
      const observer = new ResizeObserver(function() {
        positionButton(btn, mediaInfo.el);
      });
      observer.observe(mediaInfo.el);
    }

    return btn;
  }

  // Position button at bottom-right of the media element
  function positionButton(btn, el) {
    const rect = el.getBoundingClientRect();
    const scrollX = window.scrollX;
    const scrollY = window.scrollY;
    btn.style.top = (rect.bottom + scrollY - 44) + 'px';
    btn.style.left = (rect.right + scrollX - 44) + 'px';
  }

  // Show feedback animation on FAB
  function showFABFeedback(btn, type) {
    const color = type === 'success' ? '#00ff88' : '#ff3366';
    const originalBg = btn.style.background;
    btn.style.background = color;
    setTimeout(function() { btn.style.background = originalBg; }, 600);
  }

  // Inject styles for download buttons
  function injectStyles() {
    if (document.getElementById('multidown-content-styles')) return;
    const style = document.createElement('style');
    style.id = 'multidown-content-styles';
    style.textContent = `
      .multidown-download-fab {
        position: absolute !important;
        z-index: 2147483647 !important;
        pointer-events: auto !important;
      }
      .multidown-media-overlay {
        position: absolute !important;
        top: 0; left: 0; right: 0; bottom: 0;
        z-index: 2147483646 !important;
        pointer-events: none !important;
      }
    `;
    (document.head || document.documentElement).appendChild(style);
  }

  // Scan the page for media elements
  function scanPage() {
    injectStyles();

    // Video elements
    document.querySelectorAll('video').forEach(function(video) {
      const info = getMediaInfo(video, MEDIA_TYPES.VIDEO);
      if (!info) return;
      info.el = video;
      // Make video position relative so we can absolutely position the button
      const style = window.getComputedStyle(video);
      if (style.position === 'static') {
        video.style.position = 'relative';
      }
      // Overlay for click capture
      let overlay = video.parentElement && video.parentElement.querySelector('.multidown-media-overlay');
      if (!overlay) {
        overlay = document.createElement('div');
        overlay.className = 'multidown-media-overlay';
        if (video.parentElement) {
          video.parentElement.style.position = 'relative';
          video.parentElement.insertBefore(overlay, video);
        }
      }
      createDownloadButton(info);
    });

    // Audio elements
    document.querySelectorAll('audio').forEach(function(audio) {
      const info = getMediaInfo(audio, MEDIA_TYPES.AUDIO);
      if (!info) return;
      info.el = audio;
      const style = window.getComputedStyle(audio);
      if (style.position === 'static') {
        audio.style.position = 'relative';
      }
      createDownloadButton(info);
    });

    // Large images (only in visible area, debounced)
    document.querySelectorAll('img').forEach(function(img) {
      // Skip already processed
      if (img.dataset.multidownProcessed) return;
      img.dataset.multidownProcessed = 'true';
      const info = getImageInfo(img);
      if (!info) return;
      info.el = img;
      const style = window.getComputedStyle(img);
      if (style.position === 'static') {
        img.style.position = 'relative';
      }
      createDownloadButton(info);
    });
  }

  // Listen for messages from popup or background
  chrome.runtime.onMessage.addListener(function(message, sender, sendResponse) {
    if (message.action === 'get_media') {
      const media = [];
      document.querySelectorAll('video').forEach(function(el) {
        const info = getMediaInfo(el, MEDIA_TYPES.VIDEO);
        if (info) media.push(info);
      });
      document.querySelectorAll('audio').forEach(function(el) {
        const info = getMediaInfo(el, MEDIA_TYPES.AUDIO);
        if (info) media.push(info);
      });
      document.querySelectorAll('img').forEach(function(el) {
        if (el.dataset.multidownProcessed) return;
        const info = getImageInfo(el);
        if (info) media.push(info);
      });
      sendResponse({ media: media });
    }
    return true;
  });

  // Initial scan after DOM is ready
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function() {
      setTimeout(scanPage, 500);
    });
  } else {
    setTimeout(scanPage, 500);
  }

  // Re-scan on dynamic content changes (throttled)
  let scanTimeout;
  const observer = new MutationObserver(function() {
    clearTimeout(scanTimeout);
    scanTimeout = setTimeout(scanPage, 1000);
  });
  observer.observe(document.documentElement, { childList: true, subtree: true });

})();
