const closeFallback = document.getElementById("close-fallback");
closeFallback.hidden = true;
history.replaceState(null, document.title, window.location.pathname);
window.setTimeout(() => window.close(), 900);
window.setTimeout(() => {
  closeFallback.hidden = false;
}, 2200);
