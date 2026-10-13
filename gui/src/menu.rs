//! The menu bar on macOS (SwiftDM, File, Edit, View, Downloads, Window,
//! Help, as Apple's guidelines ask), and the same shortcuts with Ctrl on
//! Windows and Linux, which have no menu bar.
// Windows and Linux use only the shortcuts.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use eframe::egui;

use crate::power::AfterDone;
use crate::theme::ThemeChoice;

/// Something the menu bar or a shortcut asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    NewDownload,
    PasteLink,
    ImportLinks,
    OpenFolder,
    /// A list in the sidebar, by number (0: all downloads).
    Show(usize),
    Settings,
    Help,
    About,
    AddToBrowser,
    PauseAll,
    ResumeAll,
    ClearCompleted,
    AfterDone(AfterDone),
    CheckUpdates,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Theme(ThemeChoice),
    Open(&'static str),
}

pub const PROJECT: &str = "https://github.com/itsmrroot/SwiftDM-release";
const RELEASES: &str = "https://github.com/itsmrroot/SwiftDM-release/releases";
const ISSUES: &str = "https://github.com/itsmrroot/SwiftDM-release/issues";

/// Runs an edit command on the focused text field. The menu takes the key
/// press itself, so the field gets the same event it would have.
pub fn edit(ctx: &egui::Context, c: Command) {
    let key = |key: egui::Key, shift: bool| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers { shift, ..egui::Modifiers::COMMAND },
    };
    match c {
        Command::Cut => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut),
        Command::Copy => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy),
        Command::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
        Command::Undo => ctx.input_mut(|i| i.events.push(key(egui::Key::Z, false))),
        Command::Redo => ctx.input_mut(|i| i.events.push(key(egui::Key::Z, true))),
        Command::SelectAll => ctx.input_mut(|i| i.events.push(key(egui::Key::A, false))),
        _ => {}
    }
}

/// Shortcuts that work the same with and without a menu bar (⌘ on macOS,
/// Ctrl elsewhere). On macOS the menu bar shows them too.
fn shortcuts(ctx: &egui::Context, out: &mut Vec<Command>) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5];
    ctx.input_mut(|i| {
        for (n, key) in digits.iter().enumerate() {
            if i.consume_shortcut(&KeyboardShortcut::new(cmd, *key)) {
                out.push(Command::Show(n));
            }
        }
        let pairs = [
            (cmd, Key::N, Command::NewDownload),
            (cmd, Key::O, Command::ImportLinks),
            (cmd_shift, Key::O, Command::OpenFolder),
            (cmd, Key::Comma, Command::Settings),
            (cmd_shift, Key::P, Command::PauseAll),
            (cmd_shift, Key::R, Command::ResumeAll),
        ];
        for (mods, key, c) in pairs {
            if i.consume_shortcut(&KeyboardShortcut::new(mods, key)) {
                out.push(c);
            }
        }
        if i.consume_key(Modifiers::NONE, Key::F1) {
            out.push(Command::Help);
        }
    });
}

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

    use super::*;
    use crate::i18n::{Lang, tr};

    #[derive(Default)]
    pub struct MenuBar {
        menu: Option<Menu>,
        lang: Option<Lang>,
        ids: HashMap<MenuId, Command>,
        events: Arc<Mutex<Vec<MenuId>>>,
        handler: bool,
    }

    fn acc(mods: Modifiers, code: Code) -> Option<Accelerator> {
        Some(Accelerator::new(Some(mods), code))
    }

    impl MenuBar {
        /// The commands chosen since the last call; builds the menu bar on
        /// the first call and again when the language changes.
        pub fn commands(&mut self, ctx: &egui::Context, lang: Lang) -> Vec<Command> {
            if !self.handler {
                self.handler = true;
                let (events, ctx) = (self.events.clone(), ctx.clone());
                MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                    if let Ok(mut v) = events.lock() {
                        v.push(e.id);
                    }
                    ctx.request_repaint();
                }));
            }
            if self.lang != Some(lang) {
                self.lang = Some(lang);
                if let Err(e) = self.build() {
                    log::warn!("menu bar: {e}");
                }
            }
            let ids: Vec<MenuId> = self.events.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
            let mut out: Vec<Command> = ids.iter().filter_map(|id| self.ids.get(id).copied()).collect();
            // Keys the menu bar did not take (when a text field has focus).
            shortcuts(ctx, &mut out);
            out
        }

        fn item(&mut self, text: &str, c: Command, a: Option<Accelerator>) -> MenuItem {
            let item = MenuItem::new(text, true, a);
            self.ids.insert(item.id().clone(), c);
            item
        }

        fn build(&mut self) -> muda::Result<()> {
            self.ids.clear();
            let cmd = Modifiers::SUPER;
            let cmd_shift = Modifiers::SUPER | Modifiers::SHIFT;
            let sep = PredefinedMenuItem::separator;

            let about = self.item(tr("About SwiftDM"), Command::About, None);
            let updates = self.item(tr("Check for Updates…"), Command::CheckUpdates, None);
            let settings = self.item(&format!("{}…", tr("Settings")), Command::Settings, acc(cmd, Code::Comma));
            let app = Submenu::with_items(
                "SwiftDM",
                true,
                &[
                    &about,
                    &updates,
                    &sep(),
                    &settings,
                    &sep(),
                    &PredefinedMenuItem::services(Some(tr("Services"))),
                    &sep(),
                    &PredefinedMenuItem::hide(Some(tr("Hide SwiftDM"))),
                    &PredefinedMenuItem::hide_others(Some(tr("Hide Others"))),
                    &PredefinedMenuItem::show_all(Some(tr("Show All"))),
                    &sep(),
                    &PredefinedMenuItem::quit(Some(tr("Quit SwiftDM"))),
                ],
            )?;

            let new = self.item(&format!("{}…", tr("New download")), Command::NewDownload, acc(cmd, Code::KeyN));
            let paste = self.item(tr("Download copied link"), Command::PasteLink, acc(cmd_shift, Code::KeyV));
            let import = self.item(tr("Import links from a file…"), Command::ImportLinks, acc(cmd, Code::KeyO));
            let folder = self.item(tr("Open download folder"), Command::OpenFolder, acc(cmd_shift, Code::KeyO));
            let file = Submenu::with_items(tr("File"), true, &[&new, &paste, &import, &sep(), &folder])?;

            let undo = self.item(tr("Undo"), Command::Undo, acc(cmd, Code::KeyZ));
            let redo = self.item(tr("Redo"), Command::Redo, acc(cmd_shift, Code::KeyZ));
            let cut = self.item(tr("Cut"), Command::Cut, acc(cmd, Code::KeyX));
            let copy = self.item(tr("Copy"), Command::Copy, acc(cmd, Code::KeyC));
            let paste_text = self.item(tr("Paste"), Command::Paste, acc(cmd, Code::KeyV));
            let all = self.item(tr("Select All"), Command::SelectAll, acc(cmd, Code::KeyA));
            let edit = Submenu::with_items(tr("Edit"), true, &[&undo, &redo, &sep(), &cut, &copy, &paste_text, &all])?;

            let lists = [tr("All downloads"), tr("Downloading"), tr("Completed"), tr("Paused"), tr("Failed")];
            let digits = [Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4, Code::Digit5];
            let list_items: Vec<MenuItem> = lists
                .iter()
                .zip(digits)
                .enumerate()
                .map(|(n, (l, d))| self.item(l, Command::Show(n), acc(cmd, d)))
                .collect();
            let actual = self.item(tr("Actual Size"), Command::ZoomReset, acc(cmd, Code::Digit0));
            let zin = self.item(tr("Zoom In"), Command::ZoomIn, acc(cmd, Code::Equal));
            let zout = self.item(tr("Zoom Out"), Command::ZoomOut, acc(cmd, Code::Minus));
            let theme_items: Vec<MenuItem> =
                ThemeChoice::ALL.iter().map(|t| self.item(t.name(), Command::Theme(*t), None)).collect();
            let theme_refs: Vec<&dyn muda::IsMenuItem> =
                theme_items.iter().map(|i| i as &dyn muda::IsMenuItem).collect();
            let themes = Submenu::with_items(tr("Theme"), true, &theme_refs)?;
            let mut view_refs: Vec<&dyn muda::IsMenuItem> =
                list_items.iter().map(|i| i as &dyn muda::IsMenuItem).collect();
            let (s1, s2, s3) = (sep(), sep(), sep());
            let full = PredefinedMenuItem::fullscreen(Some(tr("Enter Full Screen")));
            view_refs.extend([&s1 as &dyn muda::IsMenuItem, &actual, &zin, &zout, &s2, &themes, &s3, &full]);
            let view = Submenu::with_items(tr("View"), true, &view_refs)?;

            let pause = self.item(tr("Pause all"), Command::PauseAll, acc(cmd_shift, Code::KeyP));
            let resume = self.item(tr("Continue all"), Command::ResumeAll, acc(cmd_shift, Code::KeyR));
            let clear = self.item(tr("Clear completed"), Command::ClearCompleted, None);
            let done_items: Vec<MenuItem> =
                AfterDone::ALL.iter().map(|a| self.item(a.name(), Command::AfterDone(*a), None)).collect();
            let done_refs: Vec<&dyn muda::IsMenuItem> = done_items.iter().map(|i| i as &dyn muda::IsMenuItem).collect();
            let when_done = Submenu::with_items(tr("When downloads are done"), true, &done_refs)?;
            let browser = self.item(&format!("{}…", tr("Add SwiftDM to your browser")), Command::AddToBrowser, None);
            let downloads = Submenu::with_items(
                tr("Downloads"),
                true,
                &[&pause, &resume, &clear, &sep(), &when_done, &sep(), &browser],
            )?;

            let window = Submenu::with_items(
                tr("Window"),
                true,
                &[
                    &PredefinedMenuItem::minimize(Some(tr("Minimize"))),
                    &PredefinedMenuItem::maximize(Some(tr("Zoom"))),
                    &sep(),
                    &PredefinedMenuItem::bring_all_to_front(Some(tr("Bring All to Front"))),
                ],
            )?;

            let help_item = self.item(tr("SwiftDM Help"), Command::Help, acc(cmd_shift, Code::Slash));
            let news = self.item(tr("What's new"), Command::Open(RELEASES), None);
            let project = self.item(tr("Project page"), Command::Open(PROJECT), None);
            let issue = self.item(tr("Report a problem"), Command::Open(ISSUES), None);
            let help = Submenu::with_items(tr("Help"), true, &[&help_item, &sep(), &news, &project, &issue])?;

            let menu = Menu::with_items(&[&app, &file, &edit, &view, &downloads, &window, &help])?;
            menu.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();
            help.set_as_help_menu_for_nsapp();
            self.menu = Some(menu);
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    use crate::i18n::Lang;

    /// No menu bar: the same shortcuts with Ctrl.
    #[derive(Default)]
    pub struct MenuBar {
        // Built like the macOS one (`MenuBar::default()`).
        _none: (),
    }

    impl MenuBar {
        pub fn commands(&mut self, ctx: &egui::Context, _lang: Lang) -> Vec<Command> {
            let mut out = Vec::new();
            shortcuts(ctx, &mut out);
            out
        }
    }
}

pub use imp::MenuBar;
