// SwiftDM: a download button on videos (shown while the mouse is on one).
(() => {
  const ext = globalThis.browser ?? globalThis.chrome;
  if (window.__swiftdm) return;
  window.__swiftdm = true;

  const t = (key) => ext.i18n.getMessage(key) || key;
  let enabled = true;
  ext.runtime.sendMessage({ type: "settings" }).then((s) => (enabled = s?.videoButton !== false)).catch(() => {});
  ext.storage.onChanged.addListener((c) => {
    if (c.videoButton) enabled = c.videoButton.newValue !== false;
  });

  // In a shadow root, so that the page's styles cannot touch it.
  const host = document.createElement("swiftdm-button");
  host.style.cssText = "all: initial; position: fixed; z-index: 2147483647; display: none;";
  const root = host.attachShadow({ mode: "closed" });
  const rtl = (ext.i18n.getUILanguage() || "").startsWith("ar");
  root.innerHTML = `
    <style>
      .bar { display: flex; gap: 2px; font: 600 12.5px system-ui, -apple-system, "Segoe UI", sans-serif;
             background: rgba(17, 17, 34, .86); border-radius: 10px; padding: 3px; box-shadow: 0 4px 14px rgba(0,0,0,.35);
             direction: ${rtl ? "rtl" : "ltr"}; }
      button { all: unset; cursor: pointer; color: #fff; padding: 6px 10px; border-radius: 8px; display: flex;
               align-items: center; gap: 6px; white-space: nowrap; }
      button:hover { background: #6366f1; }
      .main { background: #6366f1; }
      .main:hover { background: #4f51cc; }
      svg { width: 14px; height: 14px; fill: currentColor; }
      .toast { color: #fff; padding: 6px 10px; }
    </style>
    <div class="bar">
      <button class="main" data-mode="video" title="${t("buttonTitle")}">
        <svg viewBox="0 0 24 24"><path d="M13 2 5 14h6l-2 8 10-13h-7z"/></svg>${t("video")}
      </button>
      <button data-mode="audio">${t("sound")}</button>
    </div>`;
  (document.body || document.documentElement).appendChild(host);
  const bar = root.querySelector(".bar");

  let target = null;
  let hideTimer = 0;

  function place(video) {
    const r = video.getBoundingClientRect();
    if (r.width < 200 || r.height < 120) return hide();
    target = video;
    host.style.display = "block";
    host.style.top = `${Math.max(r.top + 10, 4)}px`;
    host.style.left = rtl ? `${r.left + 10}px` : "";
    host.style.right = rtl ? "" : `${Math.max(window.innerWidth - r.right + 10, 4)}px`;
  }

  function hide() {
    host.style.display = "none";
    target = null;
  }

  document.addEventListener(
    "mouseover",
    (e) => {
      if (!enabled) return;
      if (host.contains(e.target) || e.composedPath().includes(host)) {
        clearTimeout(hideTimer);
        return;
      }
      // The video itself, or a control layer drawn over it.
      const video =
        e.target.closest?.("video") ||
        [...document.querySelectorAll("video")].find((v) => {
          const r = v.getBoundingClientRect();
          return e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
        });
      clearTimeout(hideTimer);
      if (video) place(video);
      else hideTimer = setTimeout(hide, 600);
    },
    { passive: true },
  );
  window.addEventListener("scroll", () => target && place(target), { passive: true });

  root.addEventListener("click", async (e) => {
    const button = e.target.closest("button");
    if (!button || !target) return;
    e.preventDefault();
    e.stopPropagation();
    const reply = await ext.runtime
      .sendMessage({ type: "video", mode: button.dataset.mode, src: target.currentSrc || target.src, pageUrl: location.href })
      .catch(() => ({ ok: false }));
    const old = bar.innerHTML;
    bar.innerHTML = `<span class="toast">${reply?.ok ? "✓ " + t("sent") : "⚠ " + t("failed")}</span>`;
    setTimeout(() => (bar.innerHTML = old), 1800);
  });
})();
