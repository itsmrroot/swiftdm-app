// SwiftDM: a download button on videos (shown while the mouse is on one).
(() => {
  const ext = globalThis.browser ?? globalThis.chrome;
  if (window.__swiftdm) return;
  window.__swiftdm = true;

  const t = (key) => ext.i18n.getMessage(key) || key;

  // ---------------------------------------------------------------------------
  // The address of a video: on feeds (TikTok, Instagram, X…) the page's
  // address is the feed, and the video itself plays from a private blob:
  // stream — its own link is found next to it in the page.

  const SINGLE = [
    /tiktok\.com\/@[^/?#]*\/(video|photo)\/\d+/,
    /instagram\.com\/(reels?|p|tv)\/[\w-]+/,
    /(twitter|x)\.com\/[^/?#]+\/status\/\d+/,
    /youtube\.com\/(watch\?(.*&)?v=|shorts\/|live\/)[\w-]+/,
    /youtu\.be\/[\w-]+/,
    /facebook\.com\/(.*\/videos\/|reel\/|watch\/?\?v=)\d+/,
    /fb\.watch\/\w+/,
    /vimeo\.com\/(\d+|.*\/videos?\/\d+)/,
    /dailymotion\.com\/video\//,
    /reddit\.com\/r\/[^/]+\/comments\//,
    /twitch\.tv\/(videos\/\d+|[^/]+\/clip\/)/,
    /bilibili\.com\/video\//,
    /threads\.(net|com)\/@[^/]+\/post\//,
  ];
  const single = (url) => SINGLE.some((re) => re.test(url));

  /** TikTok's id of a video box: "xgwrapper-0-7353…", or an id attribute. */
  function tiktokId(node) {
    const wrapper = /xgwrapper-\d+-(\d{18,20})/.exec(node.id || "");
    if (wrapper) return wrapper[1];
    for (const attr of node.attributes ?? []) {
      if (attr.name !== "id" && !attr.name.startsWith("data-")) continue;
      const id = /^(?:\D*-)?(\d{18,20})$/.exec(attr.value);
      if (id) return id[1];
    }
    return null;
  }

  /** The link to `video` itself, or the page's address. */
  function videoUrl(video) {
    // Near the video first: a video page may also show others below it.
    const tiktok = location.hostname.endsWith("tiktok.com");
    let node = video;
    for (let depth = 0; node && depth < 25; depth++, node = node.parentElement) {
      if (tiktok) {
        const id = tiktokId(node);
        if (id) return `https://www.tiktok.com/@/video/${id}`;
      }
      // Up to the box that holds only this video: higher up, the links
      // belong to other posts of the feed.
      if ((node.querySelectorAll?.("video").length ?? 0) > 1) break;
      for (const a of node.querySelectorAll?.("a[href]") ?? []) {
        if (single(a.href)) return a.href;
      }
    }
    // The post it is in, when that links to one video only.
    const post = video.closest("article");
    if (post) {
      const links = new Set([...post.querySelectorAll("a[href]")].map((a) => a.href).filter(single));
      if (links.size === 1) return [...links][0];
    }
    return location.href;
  }

  /** The video being watched: the playing one, else the largest in view. */
  function currentVideo() {
    const visible = (v) => {
      const r = v.getBoundingClientRect();
      const w = Math.max(0, Math.min(r.right, innerWidth) - Math.max(r.left, 0));
      const h = Math.max(0, Math.min(r.bottom, innerHeight) - Math.max(r.top, 0));
      return w * h;
    };
    const videos = [...document.querySelectorAll("video")].filter((v) => visible(v) > 0);
    const playing = videos.filter((v) => !v.paused && !v.ended);
    const pool = playing.length ? playing : videos;
    return pool.sort((a, b) => visible(b) - visible(a))[0] || null;
  }

  // The popup asks which video this page is showing.
  ext.runtime.onMessage.addListener((msg, _sender, reply) => {
    if (msg?.type !== "current-video") return;
    const v = currentVideo();
    reply({ url: v ? videoUrl(v) : single(location.href) ? location.href : null, src: v?.currentSrc || null });
  });
  let enabled = true;
  // Throws, not rejects, once the extension was reloaded or removed.
  const message = (msg) => {
    try {
      return ext.runtime.sendMessage(msg);
    } catch {
      return Promise.reject(new Error("the extension was reloaded"));
    }
  };
  message({ type: "settings" }).then((s) => (enabled = s?.videoButton !== false)).catch(() => {});
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
    const reply = await message({
      type: "video",
      mode: button.dataset.mode,
      url: videoUrl(target),
      src: target.currentSrc || target.src,
      pageUrl: location.href,
    }).catch(() => ({ ok: false }));
    const old = bar.innerHTML;
    bar.innerHTML = `<span class="toast">${reply?.ok ? "✓ " + t("sent") : "⚠ " + t("failed")}</span>`;
    setTimeout(() => (bar.innerHTML = old), 1800);
  });
})();
