//! The computer around the downloads: staying awake while they run, what to
//! do when they are all done, the daily schedule, and notifications.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, Timelike};
use eframe::egui::{self, Align, Layout, RichText};
use egui_phosphor::regular as icon;
use serde::{Deserialize, Serialize};

use crate::i18n::{tr, trf};
use crate::theme::{self, Palette};

/// What happens once every download has finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AfterDone {
    #[default]
    Nothing,
    Sleep,
    ShutDown,
    Quit,
}

impl AfterDone {
    pub const ALL: [AfterDone; 4] = [AfterDone::Nothing, AfterDone::Sleep, AfterDone::ShutDown, AfterDone::Quit];

    pub fn name(self) -> &'static str {
        match self {
            AfterDone::Nothing => tr("Do nothing"),
            AfterDone::Sleep => tr("Sleep"),
            AfterDone::ShutDown => tr("Shut down"),
            AfterDone::Quit => tr("Quit SwiftDM"),
        }
    }

    /// The name in logical order, for menus the system draws (macOS).
    #[cfg(target_os = "macos")]
    pub fn name_logical(self) -> &'static str {
        match self {
            AfterDone::Nothing => crate::i18n::trl("Do nothing"),
            AfterDone::Sleep => crate::i18n::trl("Sleep"),
            AfterDone::ShutDown => crate::i18n::trl("Shut down"),
            AfterDone::Quit => crate::i18n::trl("Quit SwiftDM"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            AfterDone::Nothing => icon::CHECK_SQUARE,
            AfterDone::Sleep => icon::MOON_STARS,
            AfterDone::ShutDown => icon::POWER,
            AfterDone::Quit => icon::SIGN_OUT,
        }
    }
}

/// Puts the computer to sleep or shuts it down.
fn act(a: AfterDone) {
    let run = |program: &str, args: &[&str]| {
        let _ = swiftdm::tools::command(program).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    };
    match a {
        AfterDone::Sleep => {
            // Sleep, not hibernate (rundll32's way hibernates when that is on).
            #[cfg(windows)]
            // SAFETY: plain flags.
            unsafe {
                windows_sys::Win32::System::Power::SetSuspendState(false, false, false);
            }
            if cfg!(target_os = "macos") {
                run("pmset", &["sleepnow"]);
            } else if !cfg!(windows) {
                run("systemctl", &["suspend"]);
            }
        }
        AfterDone::ShutDown => {
            if cfg!(windows) {
                run("shutdown", &["/s", "/t", "0"]);
            } else if cfg!(target_os = "macos") {
                // Asks running apps to close, like the Apple menu does.
                run("osascript", &["-e", "tell application \"System Events\" to shut down"]);
            } else {
                run("systemctl", &["poweroff"]);
            }
        }
        AfterDone::Nothing | AfterDone::Quit => {}
    }
}

/// Keeps the computer from sleeping while held.
#[derive(Default)]
struct Awake {
    held: bool,
    #[allow(dead_code)]
    child: Option<Child>,
}

impl Awake {
    fn hold(&mut self, on: bool) {
        if on == self.held {
            return;
        }
        self.held = on;
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
            // SAFETY: plain flags; the state belongs to this (UI) thread.
            unsafe { SetThreadExecutionState(if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS }) };
        }
        #[cfg(not(windows))]
        {
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            if on {
                let pid = std::process::id().to_string();
                let mut cmd = if cfg!(target_os = "macos") {
                    let mut c = Command::new("caffeinate");
                    // Ends by itself should SwiftDM end.
                    c.args(["-i", "-w", &pid]);
                    c
                } else {
                    let mut c = Command::new("systemd-inhibit");
                    c.args([
                        "--what=sleep:idle",
                        "--who=SwiftDM",
                        "--why=Downloading",
                        "--mode=block",
                        "sh",
                        "-c",
                        // Ends by itself should SwiftDM end.
                        &format!("while kill -0 {pid} 2>/dev/null; do sleep 5; done"),
                    ]);
                    c
                };
                self.child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok();
            }
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.hold(false);
    }
}

/// A notification of the system (while SwiftDM is in the background).
pub fn notify(title: &str, body: &str) {
    let clean = |s: &str| s.replace(['"', '\\', '\n'], " ");
    if cfg!(target_os = "macos") {
        let script =
            format!("display notification \"{}\" with title \"SwiftDM\" subtitle \"{}\"", clean(body), clean(title));
        let _ = Command::new("osascript").args(["-e", &script]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    } else if cfg!(target_os = "linux") {
        let _ = Command::new("notify-send")
            .args([
                "--app-name=SwiftDM",
                "--icon=swiftdm-gui",
                "--",
                title,
                &body.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    // Windows: the taskbar button flashes instead (see app.rs).
}

/// Time of day as "HH:MM".
pub fn hhmm((h, m): (u8, u8)) -> String {
    format!("{h:02}:{m:02}")
}

pub fn parse_hhmm(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<u8>().ok()?, m.trim().parse::<u8>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// What the app should do, decided by [`Power::tick`].
pub enum Due {
    None,
    StartAll,
    PauseAll,
    /// Every download finished and the countdown ran out.
    Act(AfterDone),
}

#[derive(Default)]
pub struct Power {
    pub after_done: AfterDone,
    awake: Awake,
    /// The countdown before acting, once everything finished.
    countdown: Option<Instant>,
    was_busy: bool,
    /// Day of the year and time each scheduled time last fired (a time
    /// changed later the same day fires again).
    started_on: Option<(u32, (u8, u8))>,
    paused_on: Option<(u32, (u8, u8))>,
}

const COUNTDOWN: Duration = Duration::from_secs(30);

impl Power {
    /// Called every frame. `busy`: downloads are running or waiting;
    /// `paused`: some are paused (then they are not all done).
    pub fn tick(
        &mut self,
        ctx: &egui::Context,
        busy: bool,
        paused: bool,
        keep_awake: bool,
        start_at: Option<(u8, u8)>,
        pause_at: Option<(u8, u8)>,
    ) -> Due {
        self.awake.hold(busy && keep_awake);
        // Everything finished (not just paused): start the countdown.
        if self.was_busy && !busy && !paused && self.after_done != AfterDone::Nothing {
            self.countdown = Some(Instant::now());
        }
        if busy {
            self.countdown = None;
        }
        self.was_busy = busy;
        if let Some(at) = self.countdown {
            ctx.request_repaint_after(Duration::from_millis(500));
            if at.elapsed() >= COUNTDOWN {
                self.countdown = None;
                let a = std::mem::take(&mut self.after_done);
                return self.fire(a);
            }
        }
        // The daily schedule.
        if start_at.is_some() || pause_at.is_some() {
            ctx.request_repaint_after(Duration::from_secs(20));
            let now = Local::now();
            let (today, time) = (now.ordinal(), (now.hour() as u8, now.minute() as u8));
            if pause_at == Some(time) && self.paused_on != Some((today, time)) {
                self.paused_on = Some((today, time));
                return Due::PauseAll;
            }
            if start_at == Some(time) && self.started_on != Some((today, time)) {
                self.started_on = Some((today, time));
                return Due::StartAll;
            }
        }
        Due::None
    }

    fn fire(&mut self, a: AfterDone) -> Due {
        act(a);
        Due::Act(a)
    }

    /// The countdown dialog, while counting.
    /// `failed`: how many downloads failed (the message must not say that all went well).
    pub fn dialog(&mut self, ctx: &egui::Context, p: &Palette, failed: usize) -> Due {
        let Some(at) = self.countdown else { return Due::None };
        let left = COUNTDOWN.saturating_sub(at.elapsed()).as_secs() + 1;
        let mut due = Due::None;
        egui::Modal::new(egui::Id::new("after-done")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.after_done.icon()).size(theme::scale::ICON_XL).color(p.accent));
                let title = if failed == 0 {
                    tr("All downloads are finished").to_string()
                } else {
                    trf("Downloads are finished, {n} failed", &[("n", &failed)])
                };
                ui.label(theme::semibold(title, theme::scale::LEAD));
            });
            ui.add_space(6.0);
            let text = match self.after_done {
                AfterDone::Sleep => trf("The computer goes to sleep in {n} s.", &[("n", &left)]),
                AfterDone::ShutDown => trf("The computer shuts down in {n} s.", &[("n", &left)]),
                _ => trf("SwiftDM closes in {n} s.", &[("n", &left)]),
            };
            ui.label(RichText::new(text).size(theme::scale::BODY));
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::primary_button(ui, p, &format!("  {}  ", tr("Now")), true).clicked() {
                    self.countdown = None;
                    let a = std::mem::take(&mut self.after_done);
                    due = self.fire(a);
                }
                if theme::secondary_button(ui, tr("Cancel")).clicked() {
                    self.countdown = None;
                    self.after_done = AfterDone::Nothing;
                }
            });
        });
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_of_day() {
        assert_eq!(parse_hhmm("7:05"), Some((7, 5)));
        assert_eq!(parse_hhmm("23:59"), Some((23, 59)));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("noon"), None);
        assert_eq!(hhmm((7, 5)), "07:05");
    }
}
