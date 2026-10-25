//! Checking GitHub for a newer release and installing it.
//!
//! The installer for this system and this kind of installation is
//! downloaded, checked against the SHA-256 digest GitHub publishes, and
//! installed after the app has closed; the new version then starts:
//!
//! * Windows (installed with setup): the setup runs silently (`/S`) and
//!   starts the app again (`/R`).
//! * macOS (.app): the bundle is replaced from the disk image.
//! * Linux: an AppImage replaces itself; .deb and .rpm installs go through
//!   the package manager after the system's password dialog.
//!
//! Portable copies only get a link to the download page.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use eframe::egui::{self, Align, Layout, RichText, Ui};
use egui_phosphor::regular as icon;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use swiftdm::util::format_size;

use crate::i18n::{self, icon_label, tr, trf, trl, trlf};
use crate::jobs::{Job, Progress};
use crate::theme::{self, Palette};

/// The public repository the releases are published to (the code itself
/// is private).
pub const REPO: &str = "https://github.com/itsmrroot/SwiftDM-release";
const LATEST: &str = "https://api.github.com/repos/itsmrroot/SwiftDM-release/releases/latest";

/// The version of this program.
pub fn current() -> &'static str {
    // Development aid: pretend to be an older version to try updating.
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("SWIFTDM_PRETEND_VERSION") {
        return Box::leak(v.into_boxed_str());
    }
    env!("CARGO_PKG_VERSION")
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    /// The release page's text (from CHANGELOG.md): what is new.
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    /// `sha256:<hex>`.
    digest: Option<String>,
}

/// A newer release.
#[derive(Clone)]
pub struct Release {
    /// Without the leading `v`.
    pub version: String,
    pub page: String,
    /// What is new in it, line by line (see [`parse_notes`]).
    pub notes: Vec<Note>,
    /// The installer for this system, if it can be installed from the app
    /// (and has already been published).
    asset: Option<Asset>,
}

#[derive(Clone)]
struct Asset {
    name: String,
    url: String,
    size: u64,
    sha256: Option<String>,
}

impl Release {
    /// Whether "Update" can install it, rather than open the download page.
    pub fn installable(&self) -> bool {
        self.asset.is_some()
    }
}

/// How this copy of the app was installed.
#[derive(Clone, Debug, PartialEq)]
// Each system uses only some of these.
#[allow(dead_code)]
enum Kind {
    /// Windows, installed by the setup program into this folder.
    Setup,
    /// macOS, this .app bundle.
    Bundle(PathBuf),
    /// Linux, this AppImage file.
    AppImage(PathBuf),
    Deb,
    Rpm,
    /// Unpacked from an archive: updated by hand.
    Portable,
}

fn kind() -> Kind {
    let Ok(exe) = std::env::current_exe() else { return Kind::Portable };
    #[cfg(windows)]
    if exe.parent().is_some_and(|d| d.join("uninstall.exe").is_file()) {
        return Kind::Setup;
    }
    #[cfg(target_os = "macos")]
    if let Some(app) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))
        && exe.parent().is_some_and(|d| d.ends_with("Contents/MacOS"))
    {
        return Kind::Bundle(app.to_path_buf());
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(file) = std::env::var_os("APPIMAGE") {
            return Kind::AppImage(PathBuf::from(file));
        }
        let owned_by = |cmd: &str, arg: &str| {
            std::process::Command::new(cmd)
                .args([arg, &exe.display().to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        if owned_by("dpkg-query", "-S") {
            return Kind::Deb;
        }
        if owned_by("rpm", "-qf") {
            return Kind::Rpm;
        }
    }
    let _ = exe;
    Kind::Portable
}

/// Name of the release file that updates `kind` on this system.
fn asset_name(kind: &Kind) -> Option<String> {
    let arch = |x64: &'static str, arm: &'static str| if cfg!(target_arch = "aarch64") { arm } else { x64 };
    Some(match kind {
        Kind::Setup => format!("swiftdm-windows-{}-setup.exe", arch("x64", "arm64")),
        Kind::Bundle(_) => format!("swiftdm-macos-{}.dmg", arch("intel", "apple-silicon")),
        Kind::AppImage(_) => format!("swiftdm-linux-{}.AppImage", arch("x86_64", "arm64")),
        Kind::Deb => format!("swiftdm-linux-{}.deb", arch("x86_64", "arm64")),
        Kind::Rpm => format!("swiftdm-linux-{}.rpm", arch("x86_64", "arm64")),
        Kind::Portable => return None,
    })
}

/// `1.2.3` or `v1.2.3` as numbers; anything after the numbers is ignored.
fn parse_version(v: &str) -> Option<[u64; 3]> {
    let mut parts = v.trim().trim_start_matches('v').split(['.', '-', '+']);
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    Some(out)
}

fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("swiftdm-gui/{}", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        // A stalled download must end (and so be cancellable) some day.
        .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
        .build()
        .into()
}

/// One line of "what's new".
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    Heading(String),
    Bullet(String),
    Text(String),
}

/// The release page's text as lines to show: the markdown of the changelog
/// without its marks (`**bold**`, `` `code` ``, `[text](link)`), up to the
/// first rule ("---"), which starts the line about the downloads.
pub fn parse_notes(body: &str) -> Vec<Note> {
    let plain = |line: &str| -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(i) = rest.find('[') {
            // "[text](link)" is its text.
            match (rest[i..].find("]("), rest[i..].find(')')) {
                (Some(mid), Some(end)) if mid < end => {
                    out.push_str(&rest[..i]);
                    out.push_str(&rest[i + 1..i + mid]);
                    rest = &rest[i + end + 1..];
                }
                _ => break,
            }
        }
        out.push_str(rest);
        out.replace("**", "").replace(['`', '*'], "").trim().to_string()
    };
    let mut notes = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line == "---" || line == "***" {
            break;
        }
        if line.is_empty() {
            continue;
        }
        if let Some(h) = line.strip_prefix('#') {
            notes.push(Note::Heading(plain(h.trim_start_matches('#'))));
        } else if let Some(b) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            notes.push(Note::Bullet(plain(b)));
        } else {
            notes.push(Note::Text(plain(line)));
        }
    }
    notes
}

/// Asks GitHub for the latest release. `None` means this is the newest.
pub fn check() -> Result<Option<Release>> {
    let text = agent()
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("could not reach GitHub")?
        .body_mut()
        .read_to_string()?;
    let gh: GhRelease = serde_json::from_str(&text).context("unexpected answer from GitHub")?;
    let version = gh.tag_name.trim_start_matches('v').to_string();
    if gh.draft || gh.prerelease || !is_newer(&version, current()) {
        return Ok(None);
    }
    let asset = asset_name(&kind()).and_then(|name| {
        let a = gh.assets.into_iter().find(|a| a.name == name)?;
        Some(Asset {
            name: a.name,
            url: a.browser_download_url,
            size: a.size,
            sha256: a.digest.and_then(|d| d.strip_prefix("sha256:").map(str::to_ascii_lowercase)),
        })
    });
    Ok(Some(Release { version, page: gh.html_url, notes: parse_notes(gh.body.as_deref().unwrap_or_default()), asset }))
}

/// Downloads the installer of `release` into a new temporary folder and
/// checks it.
pub fn download(release: &Release, progress: &Progress, cancel: &AtomicBool) -> Result<PathBuf> {
    let Some(asset) = &release.asset else { bail!("no installer for this system") };
    // In SwiftDM's own folder: in the shared temporary folder, another
    // user could put something in its place.
    let dir = swiftdm::paths::data_dir().join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let name = Path::new(&asset.name).file_name().context("a bad installer name")?;
    let path = dir.join(name);

    let mut response = agent().get(&asset.url).call().context("could not download the update")?;
    let mut body = response.body_mut().with_config().limit(asset.size + 1).reader();
    let mut file = File::options().write(true).create_new(true).open(&path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 256 * 1024];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let n = body.read(&mut buf).context("the download was interrupted")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
        total += n as u64;
        progress.set(total, Some(asset.size));
    }
    file.sync_all()?;
    drop(file);
    ensure!(total == asset.size, "the download is incomplete");
    if let Some(want) = &asset.sha256 {
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        ensure!(&got == want, "the download is damaged (checksum mismatch)");
    }
    Ok(path)
}

/// Shell-quotes `s` for `sh`.
#[cfg(unix)]
fn quote(s: &Path) -> String {
    format!("'{}'", s.display().to_string().replace('\'', r"'\''"))
}

/// Runs `script` with `sh` once this process has exited, detached from it.
#[cfg(unix)]
fn after_exit(script: &str) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let pid = std::process::id();
    let script = format!("while kill -0 {pid} 2>/dev/null; do sleep 0.3; done\n{script}");
    std::process::Command::new("/bin/sh")
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .context("could not start the installer")?;
    Ok(())
}

/// Starts installing the downloaded `file`. On success the app must close
/// right away: the installation continues without it and starts the new
/// version.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub fn install(file: &Path, sha256: Option<&str>) -> Result<()> {
    match kind() {
        #[cfg(windows)]
        Kind::Setup => {
            use std::os::windows::process::CommandExt;
            // The silent setup ends a running app at once; this one first
            // pauses its downloads and saves the list (`on_exit`). So the
            // setup starts only once this process has exited, from a hidden
            // waiter that outlives it.
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let setup = file.display().to_string().replace('\'', "''");
            let script = format!(
                "Wait-Process -Id {} -ErrorAction SilentlyContinue; Start-Process -FilePath '{setup}' -ArgumentList '/S','/R'",
                std::process::id()
            );
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
                .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
                .spawn()
                .context("could not start the setup")?;
            Ok(())
        }
        #[cfg(target_os = "macos")]
        Kind::Bundle(app) => {
            let parent = app.parent().context("unexpected app location")?;
            if !writable(parent) {
                // No rights to replace the app: let the user drag it over.
                std::process::Command::new("open").arg(file).spawn()?;
                return Ok(());
            }
            let (dmg, app, staged) = (quote(file), quote(&app), quote(&app.with_extension("app.new")));
            // The disk image shows the licence, which `yes` accepts. The old
            // app is only removed once the new one is fully copied; if
            // anything fails, the disk image is opened for the user.
            after_exit(&format!(
                r#"mnt=$(mktemp -d) || exit 1
fail() {{ hdiutil detach "$mnt" -quiet 2>/dev/null; open {dmg}; exit 1; }}
yes | PAGER=cat hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$mnt" {dmg} >/dev/null 2>&1 || fail
src=$(ls -d "$mnt"/*.app | head -n 1)
[ -n "$src" ] || fail
rm -rf {staged}
ditto "$src" {staged} || fail
hdiutil detach "$mnt" -quiet
rm -rf {app} && mv {staged} {app} && open {app}"#
            ))
        }
        #[cfg(target_os = "linux")]
        Kind::AppImage(path) => {
            use std::os::unix::fs::PermissionsExt;
            let dir = path.parent().context("unexpected AppImage location")?;
            ensure!(writable(dir), "no permission to replace {}", path.display());
            // Copied next to it first, so that the switch is a rename.
            let staged = dir.join(format!(".{}.new", path.file_name().unwrap_or_default().to_string_lossy()));
            std::fs::copy(file, &staged)?;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&staged, &path)?;
            after_exit(&quote(&path))
        }
        #[cfg(target_os = "linux")]
        k @ (Kind::Deb | Kind::Rpm) => {
            use std::os::unix::fs::PermissionsExt;
            // The package manager reads it as root and as its own user.
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o644))?;
            if let Some(dir) = file.parent() {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
            }
            // The package is run as root: its checksum is checked once more
            // in that very moment, by the privileged script (the file sits in
            // a folder the user can write to).
            let want = sha256.filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()));
            let Some(want) = want else { bail!("the download cannot be checked") };
            let install = if k == Kind::Deb {
                "apt-get install -y --allow-downgrades \"$1\""
            } else {
                "if command -v dnf >/dev/null; then dnf install -y \"$1\"; \
                 elif command -v zypper >/dev/null; then zypper --non-interactive install --allow-unsigned-rpm \"$1\"; \
                 else rpm -U \"$1\"; fi"
            };
            // SAFETY: no arguments, no side effects.
            let sudo = if unsafe { libc::geteuid() } == 0 { "" } else { "pkexec " };
            let exe = std::env::current_exe()?;
            after_exit(&format!(
                "{sudo}/bin/sh -c {} sh {} >/dev/null 2>&1; {} &",
                quote(Path::new(&format!("echo \"{want}  $1\" | sha256sum -c - >/dev/null 2>&1 || exit 1; {install}"))),
                quote(file),
                quote(&exe)
            ))
        }
        _ => bail!("this copy of the app has to be updated by hand"),
    }
}

/// Whether this user may create files in `dir`.
#[cfg(unix)]
fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: `c` is a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

// ---------------------------------------------------------------------------
// In the app

/// Update checks and installs, and what the app shows of them.
#[derive(Default)]
pub struct Updater {
    checking: Option<(Job<Option<Release>>, bool)>,
    /// The newer release found, if any.
    release: Option<Release>,
    /// Checked: this is the newest version.
    latest: bool,
    download: Option<Job<PathBuf>>,
    /// In logical order (see `theme::paragraph`).
    error: Option<String>,
    dialog: bool,
}

impl Updater {
    /// Asks GitHub in the background. A `manual` check shows its result; a
    /// check at start only shows the update button if there is one.
    pub fn check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.checking.is_some() || self.download.is_some() {
            return;
        }
        self.error = None;
        self.latest = false;
        self.checking = Some((Job::spawn(ctx, |_, _| check()), manual));
    }

    /// Takes in finished work. Returns true when the app must close so that
    /// the update can be installed.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        if let Some((job, manual)) = &self.checking
            && let Some(result) = job.poll()
        {
            let manual = *manual;
            self.checking = None;
            match result {
                Ok(Some(r)) => {
                    self.release = Some(r);
                    self.dialog |= manual;
                }
                Ok(None) => self.latest = true,
                Err(e) if manual => {
                    self.error = Some(trlf(
                        "Could not check for updates: {error}",
                        &[("error", &i18n::engine(&format!("{e:#}")))],
                    ))
                }
                Err(e) => log::info!("update check failed: {e:#}"),
            }
        }
        if let Some(job) = &self.download {
            match job.poll() {
                None => ctx.request_repaint_after(Duration::from_millis(150)),
                Some(result) => {
                    let cancelled = job.stopping();
                    self.download = None;
                    // The checksum of what was offered, for the privileged install.
                    let want = self.release.as_ref().and_then(|r| r.asset.as_ref()).and_then(|a| a.sha256.clone());
                    match result.and_then(|file| {
                        // Cancelled in the moment the download finished: not installed.
                        ensure!(!cancelled, "cancelled");
                        install(&file, want.as_deref())
                    }) {
                        Ok(()) => return true,
                        Err(_) if cancelled => {}
                        Err(e) => {
                            self.error = Some(trlf(
                                "The update could not be installed: {error}",
                                &[("error", &i18n::engine(&format!("{e:#}")))],
                            ))
                        }
                    }
                }
            }
        }
        false
    }

    /// The version a newer release has, once one was found.
    pub fn available(&self) -> Option<&str> {
        self.release.as_ref().map(|r| r.version.as_str())
    }

    /// The button that announces an update, for the top bar or the sidebar.
    pub fn button(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(version) = self.available() else { return };
        let label =
            crate::i18n::icon_text(icon::ARROW_CIRCLE_UP, &trf("Update to {version}", &[("version", &version)]));
        if theme::primary_button(ui, p, &label, true).clicked() {
            self.dialog = true;
        }
    }

    /// "Check for updates" with its result, for the About page.
    pub fn status(&mut self, ui: &mut Ui, p: &Palette) {
        ui.horizontal_wrapped(|ui| {
            let busy = self.checking.is_some();
            if ui
                .add_enabled(!busy, egui::Button::new(icon_label(icon::ARROWS_CLOCKWISE, "Check for updates")))
                .clicked()
            {
                self.check(ui.ctx(), true);
            }
            if busy {
                ui.spinner();
                ui.label(RichText::new(tr("Checking for updates…")).color(p.weak));
            } else if let Some(version) = self.available() {
                let text = trf("Version {version} is available.", &[("version", &version)]);
                if ui.link(RichText::new(text).color(p.accent_text)).clicked() {
                    self.dialog = true;
                }
            } else if self.latest {
                ui.label(
                    RichText::new(icon_label(icon::CHECK_CIRCLE, "You have the latest version.")).color(p.success),
                );
            } else if let Some(e) = &self.error
                && !self.dialog
            {
                theme::paragraph(ui, e, 14.5, p.danger);
            }
        });
    }

    /// The update dialog, while open. `busy`: downloads are running.
    pub fn dialog(&mut self, ctx: &egui::Context, p: &Palette, busy: bool) {
        if !self.dialog {
            return;
        }
        let Some(release) = self.release.clone() else {
            self.dialog = false;
            return;
        };
        let modal = egui::Modal::new(egui::Id::new("update")).show(ctx, |ui| {
            ui.set_width(460.0_f32.min(ctx.content_rect().width() - 48.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::ARROW_CIRCLE_UP).size(theme::scale::ICON_XL).color(p.accent));
                ui.label(theme::semibold(tr("Update available"), theme::scale::LEAD).color(p.text));
            });
            ui.add_space(8.0);
            theme::paragraph(
                ui,
                &trlf(
                    "Version {version} is available. You have version {current}.",
                    &[("version", &release.version), ("current", &current())],
                ),
                14.5,
                p.text,
            );
            ui.hyperlink_to(icon_label(icon::LIST_BULLETS, "What's new"), &release.page);
            // What is new, right here (the page it opens has the same).
            if !release.notes.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(190.0).auto_shrink([false, true]).show(ui, |ui| {
                    for note in &release.notes {
                        match note {
                            Note::Heading(t) => {
                                ui.label(theme::semibold(t, theme::scale::BODY));
                            }
                            Note::Bullet(t) => {
                                ui.horizontal_top(|ui| {
                                    ui.label(RichText::new("•").color(p.accent_text));
                                    ui.add(egui::Label::new(RichText::new(t).size(13.5).color(p.text)).wrap());
                                });
                            }
                            Note::Text(t) => {
                                ui.add(egui::Label::new(RichText::new(t).size(13.5).color(p.weak)).wrap());
                            }
                        }
                    }
                });
            }
            ui.add_space(8.0);

            if let Some(job) = &self.download {
                let state = &job.progress;
                let bar = match state.fraction() {
                    Some(f) => egui::ProgressBar::new(f).show_percentage(),
                    None => egui::ProgressBar::new(0.0).animate(true),
                };
                ui.add(bar);
                ui.label(
                    RichText::new(trf(
                        "Downloading… {done} of {total}",
                        &[("done", &format_size(state.done())), ("total", &format_size(state.total().unwrap_or(0)))],
                    ))
                    .color(p.weak),
                );
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::secondary_button(ui, tr("Cancel")).clicked() {
                        job.stop();
                    }
                });
                return;
            }

            if !release.installable() {
                weak(ui, p, trl("This copy of the app was not installed with an installer, so it is updated by downloading the new version."));
            } else {
                weak(ui, p, trl("The app will close, install the update and start again."));
            }
            if busy {
                ui.add_space(4.0);
                theme::paragraph(ui, trl("Running downloads are paused and continue after the update."), 14.5, p.warning);
            }
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                theme::paragraph(ui, e, 14.5, p.danger);
            }
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if release.installable() {
                    if theme::primary_button(ui, p, &format!("  {}  ", tr("Update now")), true).clicked() {
                        self.error = None;
                        let r = release.clone();
                        self.download = Some(Job::spawn(ui.ctx(), move |progress, cancel| download(&r, progress, cancel)));
                    }
                } else if theme::primary_button(ui, p, &format!("  {}  ", tr("Open download page")), true).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&release.page));
                    self.dialog = false;
                }
                if theme::secondary_button(ui, tr("Later")).clicked() {
                    self.dialog = false;
                }
            });
        });
        // Closing from outside is "Later", except while downloading.
        if modal.should_close() && self.download.is_none() {
            self.dialog = false;
        }
    }
}

fn weak(ui: &mut Ui, p: &Palette, text: &str) {
    theme::paragraph(ui, text, 14.5, p.weak);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("v0.3.10", "0.3.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.3.1", "0.3.1"));
        assert!(!is_newer("0.3.0", "0.3.1"));
        assert!(!is_newer("garbage", "0.3.1"));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some([1, 2, 3]));
    }

    #[test]
    fn installers_match_the_release_files() {
        let names: Vec<String> =
            [Kind::Setup, Kind::Bundle(PathBuf::new()), Kind::AppImage(PathBuf::new()), Kind::Deb, Kind::Rpm]
                .iter()
                .filter_map(asset_name)
                .collect();
        let workflow = include_str!("../../.github/workflows/release.yml");
        for name in names {
            // The workflow names them swiftdm-<label>(-setup).<ext>.
            let label = name.trim_start_matches("swiftdm-").split(['.']).next().unwrap().trim_end_matches("-setup");
            assert!(workflow.contains(&format!("label: {label} ")), "{name}");
        }
        assert_eq!(asset_name(&Kind::Portable), None);
    }

    #[test]
    fn the_release_page_text_becomes_lines() {
        let body = "\n- **Many files:** `https://x.org/a[1-3].jpg` is three links.\n- **Change link…** (the ⋮ menu) see [the guide](https://x.org/g).\n\n## Fixes\nPlain text here.\n\n---\nDownloads for Windows: see the [install guide](https://github.com/x#-install).\n";
        assert_eq!(
            parse_notes(body),
            vec![
                Note::Bullet("Many files: https://x.org/a[1-3].jpg is three links.".into()),
                Note::Bullet("Change link… (the ⋮ menu) see the guide.".into()),
                Note::Heading("Fixes".into()),
                Note::Text("Plain text here.".into()),
            ]
        );
        assert!(parse_notes("").is_empty());
    }
}
