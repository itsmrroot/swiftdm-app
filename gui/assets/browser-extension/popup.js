// The toolbar popup: this page's video, media files the page loaded, settings.
const ext = globalThis.browser ?? globalThis.chrome;
const t = (key) => ext.i18n.getMessage(key) || key;
const $ = (id) => document.getElementById(id);

document.documentElement.dir = (ext.i18n.getUILanguage() || "").startsWith("ar") ? "rtl" : "ltr";
for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = t(el.dataset.i18n);

const size = (b) => {
  if (!b) return "";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) (b /= 1024), i++;
  return `${b.toFixed(i ? 1 : 0)} ${u[i]}`;
};

async function send(msg, button) {
  if (button) button.disabled = true;
  const reply = await ext.runtime.sendMessage({ type: "send", ...msg }).catch(() => null);
  if (button) {
    button.textContent = reply?.ok ? "✓ " + t("sent") : "⚠ " + t("failed");
    if (!reply?.ok) {
      $("setup").hidden = false;
      button.disabled = false;
    }
  }
  if (reply?.ok) setTimeout(() => window.close(), 700);
}

(async () => {
  const [tab] = await ext.tabs.query({ active: true, currentWindow: true });
  const page = tab?.url || "";
  const web = /^https?:/i.test(page);

  // Connection to the app.
  const status = $("status");
  const ping = await ext.runtime.sendMessage({ type: "ping" }).catch(() => null);
  const ok = !!ping?.ok;
  status.classList.add(ok ? "ok" : "bad");
  status.querySelector("span").textContent = ok ? t("connected") : t("notConnected");
  $("setup").hidden = ok;

  // This page: the video it is showing (on a feed, the one being watched).
  let target = page;
  if (web) {
    const found = await ext.tabs.sendMessage(tab.id, { type: "current-video" }).catch(() => null);
    if (found?.url) target = found.url;
  }
  $("page").hidden = !web;
  $("pageTitle").textContent = tab?.title || page;
  $("pageTitle").title = target;
  $("video").onclick = (e) => send({ url: target, mode: "video", referer: page, incognito: !!tab?.incognito }, e.target);
  $("audio").onclick = (e) => send({ url: target, mode: "audio", referer: page, incognito: !!tab?.incognito }, e.target);

  // The files this page offers, by kind: all of one kind with one click.
  if (web) {
    const files = await ext.tabs.sendMessage(tab.id, { type: "page-files" }).catch(() => null);
    const kinds = ["image", "video", "audio", "document", "archive", "program"].filter((k) => files?.groups?.[k]?.length);
    for (const kind of kinds) {
      const urls = files.groups[kind];
      const li = document.createElement("li");
      li.innerHTML = `<span class="name"></span><span class="meta"></span><button></button>`;
      li.querySelector(".name").textContent = t(`kind_${kind}`);
      li.querySelector(".meta").textContent = String(urls.length);
      const b = li.querySelector("button");
      b.textContent = t("downloadAll");
      b.onclick = () => send({ type: "bulk", urls, referer: page, incognito: !!tab.incognito }, b);
      $("groups").appendChild(li);
    }
    $("grab").hidden = kinds.length === 0;
  }

  // Media files the page loaded.
  const key = `tab:${tab?.id}`;
  const list = (await ext.storage.session.get(key))[key] || [];
  $("empty").hidden = list.length > 0;
  for (const m of list) {
    const li = document.createElement("li");
    const last = new URL(m.url).pathname.split("/").pop() || m.url;
    let name = last;
    try {
      name = decodeURIComponent(last);
    } catch {
      // A "%" that is not an escape: shown as it is.
    }
    li.innerHTML = `<span class="name"></span><span class="meta"></span><button></button>`;
    li.querySelector(".name").textContent = name;
    li.querySelector(".name").title = m.url;
    li.querySelector(".meta").textContent = [m.type === "stream" ? "HLS/DASH" : m.type.split("/")[1], size(m.size)]
      .filter(Boolean)
      .join(" · ");
    const b = li.querySelector("button");
    b.textContent = t("download");
    // Streams are read by yt-dlp; files are downloaded as they are.
    b.onclick = () => send({ url: m.url, mode: m.type === "stream" ? "video" : "file", referer: page, incognito: !!tab?.incognito }, b);
    $("media").appendChild(li);
  }

  // Settings.
  const s = await ext.runtime.sendMessage({ type: "settings" });
  for (const id of ["catchDownloads", "videoButton"]) {
    $(id).checked = !!s[id];
    $(id).onchange = () => ext.storage.sync.set({ [id]: $(id).checked });
  }
  // The button of the videos, on this site (it can be closed there).
  const host = web ? new URL(page).hostname : "";
  $("siteButton").parentElement.hidden = !web;
  $("siteButton").checked = !(s.hiddenSites ?? []).includes(host);
  $("siteButton").onchange = async () => {
    const now = (await ext.storage.sync.get("hiddenSites")).hiddenSites ?? [];
    const rest = now.filter((h) => h !== host);
    await ext.storage.sync.set({ hiddenSites: $("siteButton").checked ? rest : [...rest, host] });
  };
  $("minSizeMB").value = s.minSizeMB;
  $("minSizeMB").onchange = () => ext.storage.sync.set({ minSizeMB: Math.max(0, Number($("minSizeMB").value) || 0) });

  $("open").onclick = async () => {
    await ext.runtime.sendMessage({ type: "show" });
    window.close();
  };
})();
