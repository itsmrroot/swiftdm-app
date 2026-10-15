// SwiftDM browser extension — Powered by Bashar Salmo.
//
// Sends links to the SwiftDM app through native messaging: the browser
// starts SwiftDM's helper ("io.github.itsmrroot.swiftdm"), which hands the
// links to the running app (and starts it if needed). SwiftDM registers the
// helper with every browser each time it starts.

const HOST = "io.github.itsmrroot.swiftdm";
const ext = globalThis.browser ?? globalThis.chrome;
const t = (key) => ext.i18n.getMessage(key) || key;

const DEFAULTS = { catchDownloads: true, minSizeMB: 1, videoButton: true };

async function settings() {
  return { ...DEFAULTS, ...(await ext.storage.sync.get(null)) };
}

// ---------------------------------------------------------------------------
// Talking to SwiftDM

async function native(message) {
  try {
    const reply = await ext.runtime.sendNativeMessage(HOST, message);
    return reply ?? { ok: false, error: "no answer" };
  } catch (e) {
    return { ok: false, error: String(e?.message ?? e), missing: true };
  }
}

/** The cookies a page would send to `url`, as a Cookie header. */
async function cookieHeader(url) {
  try {
    const cookies = await ext.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return "";
  }
}

/**
 * Sends one link. `mode`: "auto" (SwiftDM decides), "video", "audio" or
 * "file". `ask`: show SwiftDM's choices first (quality, folder).
 */
async function sendLink({ url, mode = "auto", referer = "", filename = "", ask = true }) {
  const item = { url, mode, referer, filename, userAgent: navigator.userAgent };
  // The site's login travels with plain files (videos use SwiftDM's own).
  if (mode === "file" || mode === "auto") item.cookies = await cookieHeader(url);
  const reply = await native({ type: "download", items: [item], ask });
  await badge(reply.ok);
  return reply;
}

async function badge(ok) {
  await ext.action.setBadgeBackgroundColor({ color: ok ? "#16a34a" : "#dc2626" });
  await ext.action.setBadgeText({ text: ok ? "✓" : "!" });
  setTimeout(() => ext.action.setBadgeText({ text: "" }), 2500);
}

function usable(url) {
  return typeof url === "string" && /^https?:\/\//i.test(url);
}

// ---------------------------------------------------------------------------
// Context menus

async function createMenus() {
  // Promises, not callbacks: Firefox's `browser` ignores callbacks.
  await ext.contextMenus.removeAll();
  const add = (id, contexts) => ext.contextMenus.create({ id, contexts, title: t(id) });
  add("menuLink", ["link"]);
  add("menuMedia", ["video", "audio"]);
  add("menuImage", ["image"]);
  add("menuPage", ["page"]);
  add("menuPageAudio", ["page", "video", "audio"]);
}

ext.runtime.onInstalled.addListener(createMenus);
ext.runtime.onStartup?.addListener(createMenus);

ext.contextMenus.onClicked.addListener(async (info, tab) => {
  const page = info.pageUrl || tab?.url || "";
  switch (info.menuItemId) {
    case "menuLink":
      return sendLink({ url: info.linkUrl, referer: page });
    case "menuImage":
      return usable(info.srcUrl) && sendLink({ url: info.srcUrl, mode: "file", referer: page });
    case "menuMedia":
      // A stream in the page (blob:) is fetched by SwiftDM from the page.
      return sendLink({ url: usable(info.srcUrl) ? info.srcUrl : page, mode: "video", referer: page });
    case "menuPage":
      return sendLink({ url: page, mode: "video" });
    case "menuPageAudio":
      return sendLink({ url: page, mode: "audio" });
  }
});

// ---------------------------------------------------------------------------
// Catching the browser's own downloads

const FIVE_SECONDS = 5000;
/** Downloads handed back to the browser (SwiftDM was not reachable). */
const passThrough = new Set();

ext.downloads.onCreated.addListener(async (item) => {
  const s = await settings();
  const url = item.finalUrl || item.url;
  if (!s.catchDownloads || !usable(url) || passThrough.has(url)) return;
  if ((item.mime || "").startsWith("text/")) return;
  // Pause while asking SwiftDM; the size is usually known a moment later.
  try {
    await ext.downloads.pause(item.id);
  } catch {
    return; // already finished (a tiny file)
  }
  let size = item.fileSize || item.totalBytes || 0;
  // A short look at the size only: SwiftDM should come up at once.
  const until = Date.now() + 400;
  while (size <= 0 && Date.now() < until) {
    await new Promise((r) => setTimeout(r, 100));
    const [now] = await ext.downloads.search({ id: item.id });
    size = now?.totalBytes || now?.fileSize || 0;
  }
  if (size > 0 && size < s.minSizeMB * 1024 * 1024) {
    return ext.downloads.resume(item.id).catch(() => {});
  }
  const filename = (item.filename || "").split(/[\\/]/).pop();
  // SwiftDM comes to the front with its choices (name, folder) at once.
  const reply = await sendLink({ url, mode: "file", referer: item.referrer || "", filename, ask: true });
  if (reply.ok) {
    await ext.downloads.cancel(item.id).catch(() => {});
    await ext.downloads.erase({ id: item.id }).catch(() => {});
  } else {
    // SwiftDM is not there: the browser downloads it after all.
    passThrough.add(url);
    setTimeout(() => passThrough.delete(url), FIVE_SECONDS);
    ext.downloads.resume(item.id).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// Video and audio files a page loads (shown in the popup)

const MEDIA_EXT = /\.(mp4|webm|mkv|mov|m4v|mp3|m4a|aac|ogg|opus|flac|wav|m3u8|mpd)(\?|$)/i;
const SKIP_HOSTS = /(googlevideo\.com|fbcdn\.net|cdninstagram\.com|tiktokcdn|twimg\.com|akamaihd\.net\/.*\.ts)/i;

ext.webRequest.onResponseStarted.addListener(
  async (d) => {
    if (d.tabId < 0 || d.statusCode >= 300 || SKIP_HOSTS.test(d.url)) return;
    const header = (n) => d.responseHeaders?.find((h) => h.name.toLowerCase() === n)?.value || "";
    const type = header("content-type").toLowerCase();
    const playlist = /mpegurl|dash\+xml/.test(type) || /\.(m3u8|mpd)(\?|$)/i.test(d.url);
    const media = /^(video|audio)\//.test(type) && !/mp2t/.test(type);
    if (!playlist && !media && !MEDIA_EXT.test(d.url)) return;
    const size = Number(header("content-length")) || 0;
    // Small pieces of a stream are not worth listing.
    if (!playlist && size > 0 && size < 300 * 1024) return;
    const key = `tab:${d.tabId}`;
    const list = (await ext.storage.session.get(key))[key] || [];
    if (list.some((m) => m.url === d.url)) return;
    list.unshift({ url: d.url, type: playlist ? "stream" : type.split(";")[0] || "media", size });
    await ext.storage.session.set({ [key]: list.slice(0, 25) });
    ext.action.setBadgeBackgroundColor({ color: "#6366f1" });
    ext.action.setBadgeText({ tabId: d.tabId, text: String(Math.min(list.length, 99)) });
  },
  { urls: ["<all_urls>"] },
  ["responseHeaders"],
);

// A new page starts a new list.
ext.tabs.onUpdated.addListener((tabId, change) => {
  if (change.url) {
    ext.storage.session.remove(`tab:${tabId}`);
    ext.action.setBadgeText({ tabId, text: "" });
  }
});
ext.tabs.onRemoved.addListener((tabId) => ext.storage.session.remove(`tab:${tabId}`));

// ---------------------------------------------------------------------------
// Messages from the popup and the video button

ext.runtime.onMessage.addListener((msg, sender, reply) => {
  (async () => {
    switch (msg?.type) {
      case "ping":
        return native({ type: "ping" });
      case "show":
        return native({ type: "show" });
      case "send":
        return sendLink(msg);
      case "video": {
        // The button on a video: its own link as the page found it (on a
        // feed, the post it is in), unless the video is a plain file.
        const page = sender.tab?.url || msg.pageUrl;
        const direct = usable(msg.src) && MEDIA_EXT.test(msg.src) && !/m3u8|mpd/i.test(msg.src);
        const url = direct ? msg.src : usable(msg.url) ? msg.url : page;
        return sendLink({ url, mode: msg.mode || "video", referer: page });
      }
      case "settings":
        return settings();
    }
  })().then(reply);
  return true; // answers asynchronously
});
