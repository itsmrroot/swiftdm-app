//! Desktop app for SwiftDM — Powered by Bashar Salmo.

// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod add;
mod app;
mod helpers;
mod jobs;
mod list;
mod settings;
mod text;
mod theme;
mod thumbs;
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

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title("SwiftDM")
        .with_app_id("swiftdm")
        .with_inner_size(app::DEFAULT_SIZE)
        .with_min_inner_size(app::MIN_SIZE)
        .with_icon(icon());
    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        renderer: eframe::Renderer::Wgpu,
        centered: true,
        ..Default::default()
    };
    eframe::run_native("SwiftDM", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
