<div align="center">

<img src="gui/assets/icon.png" width="96" alt="SwiftDM">

# SwiftDM

**Download videos, music and files from any link — 1800+ sites and any direct file.**
Desktop app for Windows, macOS and Linux, plus the `swiftdm` command line. Powered by Bashar Salmo.

Downloads for users: **[itsmrroot/SwiftDM-release](https://github.com/itsmrroot/SwiftDM-release/releases/latest)** (this repository is private).

</div>

## What it does

- **Video sites** — YouTube, TikTok, Instagram, X, Facebook, SoundCloud, Vimeo, Reddit, Twitch and 1800+ more through
  [yt-dlp](https://github.com/yt-dlp/yt-dlp): pick the quality (4K … 240p) or keep the sound only as MP3, playlists
  and channels with per-item selection, cover art and details embedded.
- **Any file link** — up to 32 connections per file; a connection that finishes takes over half of the slowest
  remaining part, so all stay busy. Pause, quit, and continue later from where it stopped; lost or stalled
  connections reconnect on their own.
- **No Python, nothing to set up** — the app installs its helpers (yt-dlp, FFmpeg, Deno) from their official
  sources into its own folder, checks their SHA-256, and keeps yt-dlp up to date.
- **Logins** — uses the login of a browser (Chrome, Firefox, Safari, Edge, Brave, …) for private or age-restricted
  videos, and tries it automatically when a site refuses.
- Copied links are offered for download, `Ctrl/⌘+V` anywhere adds a link, notifications when downloads finish,
  queue with downloads-at-once and a global speed limit, light / dark / night themes, Arabic titles shown correctly.
- Updates itself from the public release repository (Windows setup, macOS app, AppImage, .deb, .rpm).

## Layout

| Path | |
|---|---|
| `src/http.rs` | segmented, resumable HTTP downloads |
| `src/media.rs` | yt-dlp: inspecting links, downloads with progress, friendly errors |
| `src/tools.rs` | installing and finding yt-dlp, FFmpeg, Deno |
| `src/manager.rs` | the download list: queue, persistence, retries with browser logins, fallbacks |
| `src/main.rs` | the `swiftdm` command line |
| `gui/` | the desktop app (egui / eframe) |
| `tests/` | downloads against a local test server |

## Build

```sh
cargo run -p swiftdm-gui                 # the app
cargo run -p swiftdm -- https://…        # the command line
cargo test --workspace
```

Linux needs the usual graphics headers to build the app (`libxkbcommon-dev`, `libgl1-mesa-dev` or Vulkan).

## Release

Push a tag `vX.Y.Z` (after raising the version in both `Cargo.toml` files). `.github/workflows/release.yml` builds
every system and publishes the archives and installers to
[SwiftDM-release](https://github.com/itsmrroot/SwiftDM-release) with the `RELEASES_TOKEN` secret (a token with
*Contents: read and write* on that repository), and to this repository.

## License

MIT © 2026 Bashar Salmo. Noto Sans Arabic: SIL Open Font License (`gui/assets/fonts/OFL.txt`).
