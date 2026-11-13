// 浏览器只允许脚本关闭「由脚本打开」的窗口。命令行交给系统浏览器打开的标签页
// 没有 opener，Chrome/Edge/Firefox 会静默拒绝 window.close()，控制台提示
// "Scripts may close only the windows that were opened by them"；因此关闭只能
// 尽力而为，页面文案不依赖它成功。
function attemptClose() {
  try {
    window.close();
  } catch (ignored) {
    // 被拒绝时页面保持可读，关闭方式已在卡片正文里说明。
  }
}

// 授权码与 state 已交给终端，地址栏不再保留它们。
history.replaceState(null, document.title, window.location.pathname);

attemptClose();
// 个别内嵌浏览器只允许在 load 之后关闭自身，这里补一次尝试。
window.addEventListener("load", attemptClose, { once: true });
