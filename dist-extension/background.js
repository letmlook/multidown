const HOST_NAME = "com.multidown.app";

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "multidown-link",
    title: "使用 Multidown 下载链接",
    contexts: ["link"],
  });
  chrome.contextMenus.create({
    id: "multidown-page",
    title: "使用 Multidown 下载此页面",
    contexts: ["page"],
  });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  let url = "";
  if (info.menuItemId === "multidown-link" && info.linkUrl) {
    url = info.linkUrl;
  } else if (info.menuItemId === "multidown-page" && tab?.url) {
    url = tab.url;
  }
  if (!url || (!url.startsWith("http://") && !url.startsWith("https://"))) {
    return;
  }
  try {
    const port = chrome.runtime.connectNative(HOST_NAME);
    port.onMessage.addListener((response) => {
      if (response && response.success) {
        console.log("Multidown:", response.message);
      } else {
        console.warn("Multidown:", response?.message || "未知错误");
      }
    });
    port.postMessage({ url });
  } catch (e) {
    console.warn("Multidown 扩展: 无法连接 Native Host。请确保已安装 Multidown 并已注册 Native Messaging Host。", e);
  }
});
