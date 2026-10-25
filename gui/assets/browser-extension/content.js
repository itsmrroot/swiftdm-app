// SwiftDM: a download button on videos (shown while the mouse is on one).
(() => {
  const ext = globalThis.browser ?? globalThis.chrome;
  if (window.__swiftdm) return;
  window.__swiftdm = true;

  const t = (key) => ext.i18n.getMessage(key) || key;
  // After the extension was reloaded or removed, this page's script has lost
  // its connection: the words are looked up safely.
  const say = (key) => {
    try {
      return t(key);
    } catch {
      return key;
    }
  };

  // ---------------------------------------------------------------------------
  // The address of a video: on feeds (TikTok, Instagram, X…) the page's
  // address is the feed, and the video itself plays from a private blob:
  // stream — its own link is found next to it in the page.

  // The host is anchored: "dropbox.com/..." must not count as "x.com", nor a
  // query that merely mentions "tiktok.com/@a/video/1".
  const onSite = (name) => `^https?:\\/\\/([\\w-]+\\.)*${name}`;
  const SINGLE = [
    new RegExp(`${onSite("tiktok\\.com")}\\/@[^/?#]*\\/(video|photo)\\/\\d+`),
    new RegExp(`${onSite("instagram\\.com")}\\/(reels?|p|tv)\\/[\\w-]+`),
    new RegExp(`${onSite("(twitter|x)\\.com")}\\/[^/?#]+\\/status\\/\\d+`),
    new RegExp(`${onSite("youtube\\.com")}\\/(watch\\?(.*&)?v=|shorts\\/|live\\/)[\\w-]+`),
    new RegExp(`${onSite("youtu\\.be")}\\/[\\w-]+`),
    new RegExp(`${onSite("facebook\\.com")}\\/(.*\\/videos\\/|reel\\/|watch\\/?\\?v=)\\d+`),
    new RegExp(`${onSite("fb\\.watch")}\\/\\w+`),
    new RegExp(`${onSite("vimeo\\.com")}\\/(\\d+|.*\\/videos?\\/\\d+)`),
    new RegExp(`${onSite("dailymotion\\.com")}\\/video\\/`),
    new RegExp(`${onSite("reddit\\.com")}\\/r\\/[^/]+\\/comments\\/`),
    new RegExp(`${onSite("twitch\\.tv")}\\/(videos\\/\\d+|[^/]+\\/clip\\/)`),
    new RegExp(`${onSite("bilibili\\.com")}\\/video\\/`),
    new RegExp(`${onSite("threads\\.(net|com)")}\\/@[^/]+\\/post\\/`),
  ];
  const single = (url) => SINGLE.some((re) => re.test(url));

  /** TikTok's id of a video box: only "xgwrapper-0-7353…" (the other long numbers in a page are users', sounds', …). */
  function tiktokId(node) {
    return /xgwrapper-\d+-(\d{18,20})/.exec(node.id || "")?.[1] ?? null;
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

  // The files a page offers, by kind: for "download all of these".
  const KINDS = [
    ["image", /\.(jpe?g|png|gif|webp|avif|bmp|tiff?)$/i],
    ["video", /\.(mp4|webm|mkv|mov|m4v|avi|flv|wmv)$/i],
    ["audio", /\.(mp3|m4a|aac|ogg|opus|flac|wav)$/i],
    ["document", /\.(pdf|docx?|xlsx?|pptx?|txt|epub|csv|odt|ods|rtf)$/i],
    ["archive", /\.(zip|rar|7z|tar|gz|bz2|xz|tgz|iso|dmg)$/i],
    ["program", /\.(exe|msi|apk|deb|rpm|pkg|appimage|jar)$/i],
  ];
  const kindOf = (url) => {
    try {
      const path = new URL(url).pathname;
      return KINDS.find(([, re]) => re.test(path))?.[0] ?? null;
    } catch {
      return null;
    }
  };

  function pageFiles() {
    const found = new Map();
    const add = (url, kind) => {
      if (!/^https?:\/\//i.test(url || "")) return;
      const k = kind || kindOf(url);
      if (!k) return;
      if (!found.has(k)) found.set(k, new Set());
      try {
        const u = new URL(url);
        if (url.length > 2048) return;
        u.hash = "";
        found.get(k).add(u.href);
      } catch {
        // Not an address.
      }
    };
    for (const a of document.querySelectorAll("a[href]")) add(a.href);
    // Pictures as they are shown (not icons), also when the address has no ".jpg".
    for (const img of document.images) {
      // A picture that is a link to a larger one: the larger one is the file.
      if (img.closest("a[href]") && kindOf(img.closest("a[href]").href) === "image") continue;
      const loaded = img.complete && (img.naturalWidth >= 120 || img.naturalHeight >= 120);
      const lazy = !img.complete && img.loading === "lazy";
      if (loaded || lazy) add(img.currentSrc || img.src || img.dataset.src, "image");
    }
    for (const v of document.querySelectorAll("video, video source")) add(v.currentSrc || v.src, "video");
    for (const a of document.querySelectorAll("audio, audio source")) add(a.currentSrc || a.src, "audio");
    const groups = {};
    for (const [kind, urls] of found) groups[kind] = [...urls].slice(0, 500);
    return { groups };
  }

  // The element last right-clicked: "download this video" is about that one.
  let clicked = null;
  document.addEventListener("contextmenu", (e) => (clicked = e.target), { capture: true, passive: true });

  // The popup asks which video this page is showing.
  ext.runtime.onMessage.addListener((msg, _sender, reply) => {
    if (msg?.type === "page-files") {
      reply(pageFiles());
      return;
    }
    if (msg?.type !== "current-video") return;
    // The right-clicked video (when it was one), else the one being watched.
    const asked = clicked?.closest?.("video") || (msg.context ? clicked?.querySelector?.("video") : null);
    const v = (msg.context && asked) || currentVideo();
    reply({ url: v ? videoUrl(v) : single(location.href) ? location.href : null, src: v?.currentSrc || null });
  });
  let enabled = true;
  /** The sites where the person closed the button. */
  let hiddenSites = [];
  const hiddenHere = () => hiddenSites.includes(location.hostname);
  // Throws, not rejects, once the extension was reloaded or removed.
  const message = (msg) => {
    try {
      return ext.runtime.sendMessage(msg);
    } catch {
      return Promise.reject(new Error("the extension was reloaded"));
    }
  };
  message({ type: "settings" })
    .then((s) => {
      enabled = s?.videoButton !== false;
      hiddenSites = s?.hiddenSites ?? [];
    })
    .catch(() => {});
  ext.storage.onChanged.addListener((c) => {
    if (c.videoButton) enabled = c.videoButton.newValue !== false;
    if (c.hiddenSites) {
      hiddenSites = c.hiddenSites.newValue ?? [];
      if (hiddenHere()) hide();
    }
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
      .close { padding: 6px 7px; opacity: .7; }
      .close:hover { opacity: 1; background: #dc2626; }
      .toast { color: #fff; padding: 6px 10px; }
    </style>
    <div class="bar">
      <button class="main" data-mode="video" title="${t("buttonTitle")}">
        <svg viewBox="0 0 24 24"><path d="M13 2 5 14h6l-2 8 10-13h-7z"/></svg>${t("video")}
      </button>
      <button data-mode="audio">${t("sound")}</button>
      <button class="close" data-close title="${t("hideTitle")}" aria-label="${t("hideTitle")}">
        <svg viewBox="0 0 24 24"><path d="M6.4 5 12 10.6 17.6 5 19 6.4 13.4 12 19 17.6 17.6 19 12 13.4 6.4 19 5 17.6 10.6 12 5 6.4z"/></svg>
      </button>
    </div>`;
  (document.body || document.documentElement).appendChild(host);
  const bar = root.querySelector(".bar");
  const original = bar.innerHTML;
  let sending = false;

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
      if (!enabled || hiddenHere()) return;
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
    // Capture: a player (YouTube's) may stop the event before it bubbles up.
    { passive: true, capture: true },
  );
  // Scrolling inside a page's own scroll container does not bubble: capture.
  const follow = () => {
    if (!target) return;
    const r = target.getBoundingClientRect();
    // Gone from the page, or scrolled out of view: the bar goes too.
    if (!target.isConnected || r.bottom < 0 || r.top > window.innerHeight) hide();
    else place(target);
  };
  window.addEventListener("scroll", follow, { passive: true, capture: true });
  window.addEventListener("resize", follow, { passive: true });

  root.addEventListener("click", async (e) => {
    const button = e.target.closest("button");
    if (!button) return;
    e.preventDefault();
    e.stopPropagation();
    if (button.hasAttribute("data-close")) {
      // Closed for this site; the popup brings it back.
      hiddenSites = [...new Set([...hiddenSites, location.hostname])];
      hide();
      try {
        await ext.storage.sync.set({ hiddenSites });
      } catch {
        // Hidden until the page is reloaded at least.
      }
      return;
    }
    if (!target || sending) return;
    sending = true;
    const reply = await message({
      type: "video",
      mode: button.dataset.mode,
      url: videoUrl(target),
      src: target.currentSrc || target.src,
      pageUrl: location.href,
    }).catch(() => ({ ok: false }));
    bar.innerHTML = `<span class="toast">${reply?.ok ? "✓ " + say("sent") : "⚠ " + say("failed")}</span>`;
    setTimeout(() => {
      bar.innerHTML = original;
      sending = false;
    }, 1800);
  });
})();
