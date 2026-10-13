//! Video thumbnails, fetched in the background and kept for the session.

use std::collections::HashMap;
use std::io::Read;
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

fn fetch(url: &str) -> Result<ColorImage> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(swiftdm::http::USER_AGENT)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut bytes = Vec::new();
    agent.get(url).call()?.body_mut().as_reader().take(8 * 1024 * 1024).read_to_end(&mut bytes)?;
    let img = image::load_from_memory(&bytes).context("not an image")?.thumbnail(320, 180).to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}
