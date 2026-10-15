//! Desktop app for SwiftDM — Powered by Bashar Salmo.

// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod add;
mod app;
mod browser;
mod help;
mod helpers;
mod i18n;
mod instance;
mod jobs;
mod list;
mod logfile;
mod menu;
mod power;
mod settings;
mod text;
mod theme;
mod thumbs;
mod translations;
mod update;

use eframe::egui;

fn icon() -> egui::IconData {
    image::load_from_memory(include_bytes!("../assets/icon.png"))
        .map(|i| {
            let i = i.to_rgba8();
            egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() }
        })
        .unwrap_or_default()
}

/// Which graphics to draw with. The GPU (wgpu) first; OpenGL where that
/// cannot start — e.g. older Intel graphics on Windows — remembered in
/// SwiftDM's folder. `SWIFTDM_RENDERER=glow` forces OpenGL.
fn renderer_file() -> std::path::PathBuf {
    swiftdm::paths::data_dir().join("renderer")
}

fn renderer() -> eframe::Renderer {
    let wanted = std::env::var("SWIFTDM_RENDERER").ok().or_else(|| std::fs::read_to_string(renderer_file()).ok());
    match wanted.as_deref().map(str::trim) {
        Some("glow") => eframe::Renderer::Glow,
        _ => eframe::Renderer::Wgpu,
    }
}

fn main() -> eframe::Result {
    // Started by a browser as the extension's helper: no window.
    if browser::launched_as_helper() {
        browser::run_helper();
    }
    logfile::start();
    // Links given when starting ("Open with", scripts, browser add-ons).
    let links: Vec<String> = std::env::args().skip(1).filter(|a| swiftdm::util::is_link(a)).collect();
    let inbox = match instance::start(&links) {
        instance::Start::First(inbox) => inbox,
        // The running SwiftDM took the links.
        instance::Start::Handed => {
            log::info!("swiftdm: handed over to the running SwiftDM");
            return Ok(());
        }
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title("SwiftDM")
        // The name of the desktop file (and icon) of the Linux packages, so that
        // the window belongs to its launcher.
        .with_app_id("swiftdm-gui")
        .with_inner_size(app::DEFAULT_SIZE)
        .with_min_inner_size(app::MIN_SIZE)
        // A normal window, not a maximized one (see `App::fit_to_screen`).
        .with_maximized(false)
        .with_icon(icon());
    // A separate data folder (tests, portable use) keeps its own settings.
    let separate = std::env::var_os("SWIFTDM_DATA_DIR").map(|d| std::path::PathBuf::from(d).join("app"));
    let renderer = renderer();
    log::info!("swiftdm: drawing with {renderer:?}");
    let options = eframe::NativeOptions {
        viewport,
        persistence_path: separate,
        // Not the last position and state: a window remembered on a screen
        // that is gone opens out of sight, and a maximized one stays so.
        persist_window: false,
        renderer,
        centered: true,
        // The app's own menu bar (menu.rs) replaces winit's default one,
        // which would otherwise be put back after launch.
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(|b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_default_menu(false);
        })),
        ..Default::default()
    };
    let result =
        eframe::run_native("SwiftDM", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, inbox, links)))));
    if let Err(e) = &result {
        log::error!("swiftdm: the window could not be shown: {e}");
        if renderer == eframe::Renderer::Wgpu {
            // Try again with OpenGL, in a new process (a process can open its
            // window system only once).
            let _ = std::fs::write(renderer_file(), "glow");
            let _ = std::fs::remove_file(swiftdm::paths::data_dir().join("instance-port"));
            if let Ok(exe) = std::env::current_exe()
                && std::process::Command::new(exe).args(std::env::args().skip(1)).spawn().is_ok()
            {
                log::info!("swiftdm: started again with OpenGL");
                return Ok(());
            }
        }
    }
    result
}
