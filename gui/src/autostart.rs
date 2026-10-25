//! Starting SwiftDM when the person signs in (a download manager that is
//! there when the browser asks, and finishes the downloads of last night).
//! It starts minimized: no window in the way.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// What the program is started with at sign-in: no window.
pub const ARG: &str = "--minimized";

/// The program to start: the AppImage itself, not the folder it unpacks to.
fn program() -> Result<String> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(appimage).display().to_string());
    }
    Ok(std::env::current_exe().context("the program's own place is unknown")?.display().to_string())
}

/// An XDG autostart entry (Linux).
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn desktop_entry(exe: &str) -> String {
    // A path with spaces is quoted, as the specification asks.
    let exec = if exe.contains(' ') { format!("\"{exe}\"") } else { exe.to_string() };
    format!(
        "[Desktop Entry]\nType=Application\nName=SwiftDM\nComment=Download manager\nExec={exec} {ARG}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// A launch agent (macOS).
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
fn launch_agent(exe: &str) -> String {
    let escaped = exe.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>io.github.itsmrroot.swiftdm</string>\n  <key>ProgramArguments</key>\n  <array><string>{escaped}</string><string>{ARG}</string></array>\n  <key>RunAtLoad</key><true/>\n</dict>\n</plist>\n"
    )
}

/// The value of the registry's Run entry (Windows).
#[cfg_attr(not(any(test, windows)), allow(dead_code))]
fn run_value(exe: &str) -> String {
    format!("\"{exe}\" {ARG}")
}

#[cfg(target_os = "linux")]
fn entry_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("autostart").join("swiftdm.desktop"))
}

#[cfg(target_os = "macos")]
fn entry_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/LaunchAgents/io.github.itsmrroot.swiftdm.plist"))
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Whether SwiftDM is set to start at sign-in.
pub fn is_on() -> bool {
    #[cfg(windows)]
    {
        swiftdm::tools::command("reg")
            .args(["query", RUN_KEY, "/v", "SwiftDM"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        entry_path().is_some_and(|p| p.is_file())
    }
}

/// Sets SwiftDM to start (or not) at sign-in.
pub fn set(on: bool) -> Result<()> {
    #[cfg(windows)]
    {
        let status = if on {
            swiftdm::tools::command("reg")
                .args(["add", RUN_KEY, "/v", "SwiftDM", "/t", "REG_SZ", "/d", &run_value(&program()?), "/f"])
                .status()
        } else {
            swiftdm::tools::command("reg").args(["delete", RUN_KEY, "/v", "SwiftDM", "/f"]).status()
        }
        .context("the registry could not be changed")?;
        // Deleting what is not there is no failure.
        anyhow::ensure!(status.success() || !on, "the registry could not be changed");
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let path = entry_path().context("the place for startup entries is unknown")?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            };
        }
        let exe = program()?;
        #[cfg(target_os = "linux")]
        let text = desktop_entry(&exe);
        #[cfg(target_os = "macos")]
        let text = launch_agent(&exe);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_start_the_program_minimized() {
        assert!(desktop_entry("/opt/SwiftDM/swiftdm-gui").contains("Exec=/opt/SwiftDM/swiftdm-gui --minimized\n"));
        assert!(
            desktop_entry("/home/a b/SwiftDM.AppImage").contains("Exec=\"/home/a b/SwiftDM.AppImage\" --minimized\n")
        );
        let plist = launch_agent("/Applications/SwiftDM.app/Contents/MacOS/swiftdm-gui");
        assert!(plist.contains(
            "<string>/Applications/SwiftDM.app/Contents/MacOS/swiftdm-gui</string><string>--minimized</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
        assert!(launch_agent("/a&b/<x>").contains("/a&amp;b/&lt;x&gt;"));
        assert_eq!(
            run_value(r"C:\Program Files\SwiftDM\swiftdm-gui.exe"),
            r#""C:\Program Files\SwiftDM\swiftdm-gui.exe" --minimized"#
        );
    }
}
