//! Thumbnails, fetched (videos) or read from the saved file (pictures) in
//! the background and kept for the session.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use crate::jobs::Job;

/// Thumbnails fetched at the same time.
const PARALLEL: usize = 4;

enum State {
    Loading(Job<ColorImage>),
    Ready(TextureHandle),
    Failed,
}

#[derive(Default)]
pub struct Thumbs {
    map: HashMap<String, State>,
}

impl Thumbs {
    /// The thumbnail at `url`, starting to fetch it if needed.
    pub fn get(&mut self, ctx: &egui::Context, url: &str) -> Option<TextureHandle> {
        match self.map.get(url) {
            Some(State::Ready(t)) => return Some(t.clone()),
            Some(_) => return None,
            None => {}
        }
        let loading = self.map.values().filter(|s| matches!(s, State::Loading(_))).count();
        if loading < PARALLEL {
            let u = url.to_string();
            self.map.insert(url.to_string(), State::Loading(Job::spawn(ctx, move |_, _| fetch(&u))));
        }
        None
    }

    /// The preview of a picture saved at `path`.
    pub fn get_file(&mut self, ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
        self.get(ctx, &format!("{FILE}{}", path.display()))
    }

    /// Takes in fetched thumbnails.
    pub fn poll(&mut self, ctx: &egui::Context) {
        for (url, state) in self.map.iter_mut() {
            if let State::Loading(job) = state
                && let Some(result) = job.poll()
            {
                *state = match result {
                    Ok(img) => State::Ready(ctx.load_texture(format!("thumb:{url}"), img, TextureOptions::LINEAR)),
                    Err(e) => {
                        log::info!("thumbnail {url}: {e:#}");
                        State::Failed
                    }
                };
            }
        }
    }
}

/// Marks a key that is a file on this computer, not an address.
const FILE: &str = "file:";
/// Larger pictures are not decoded for a small preview.
const MAX_FILE: u64 = 64 * 1024 * 1024;

fn to_color(bytes: &[u8]) -> Result<ColorImage> {
    let img = image::load_from_memory(bytes).context("not an image")?.thumbnail(320, 180).to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

fn fetch(url: &str) -> Result<ColorImage> {
    if let Some(path) = url.strip_prefix(FILE) {
        let file = std::fs::File::open(path).context("the file is not there")?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE).read_to_end(&mut bytes)?;
        return to_color(&bytes);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(swiftdm::http::USER_AGENT)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut bytes = Vec::new();
    agent.get(url).call()?.body_mut().as_reader().take(8 * 1024 * 1024).read_to_end(&mut bytes)?;
    to_color(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_picture_gives_a_preview() {
        let dir = std::env::temp_dir().join(format!("swiftdm-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        image::RgbaImage::from_pixel(1000, 500, image::Rgba([200, 30, 30, 255])).save(&path).unwrap();
        let img = fetch(&format!("{FILE}{}", path.display())).unwrap();
        // Shrunk to fit 320×180, in proportion.
        assert_eq!(img.size, [320, 160]);
        // Not a picture, and not there.
        std::fs::write(dir.join("b.jpg"), "text").unwrap();
        assert!(fetch(&format!("{FILE}{}", dir.join("b.jpg").display())).is_err());
        assert!(fetch(&format!("{FILE}{}", dir.join("none.png").display())).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
