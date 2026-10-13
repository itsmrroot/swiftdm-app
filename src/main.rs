//! `swiftdm` — the command line. Powered by Bashar Salmo.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use swiftdm::manager::{Manager, Settings, Source, Status};
use swiftdm::media::{self, Found, Mode};
use swiftdm::tools::{self, Tool};
use swiftdm::util::{format_eta, format_size, format_speed};

#[derive(Parser)]
#[command(
    name = "swiftdm",
    version,
    about = "Download videos, music and files from any link — 1800+ sites and any direct file.",
    after_help = "Examples:\n  swiftdm https://www.youtube.com/watch?v=…\n  swiftdm -a -k 320 https://soundcloud.com/…\n  swiftdm -q 720 -o ~/Videos https://vimeo.com/…\n  swiftdm -c 16 https://example.com/big.iso\n  swiftdm info https://www.tiktok.com/@…\n  swiftdm helpers install\n\nPowered by Bashar Salmo.",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    get: Get,
}

#[derive(Subcommand)]
enum Command {
    /// Show what is behind a link: title, length, qualities, playlist items.
    Info {
        url: String,
        /// Use the login of this browser (chrome, firefox, safari, edge, brave).
        #[arg(long)]
        cookies: Option<String>,
    },
    /// Show, install, update or remove the helper programs (yt-dlp, FFmpeg, Deno).
    Helpers {
        #[command(subcommand)]
        action: Option<HelperAction>,
    },
}

#[derive(Subcommand)]
enum HelperAction {
    /// Install (or update) helpers: all of them, or the ones named.
    Install { names: Vec<String> },
    /// Remove SwiftDM's own copies of helpers.
    Remove { names: Vec<String> },
}

#[derive(Args)]
struct Get {
    /// Links to download.
    urls: Vec<String>,
    /// Folder to save to [default: Downloads/SwiftDM].
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Save the sound only, as MP3.
    #[arg(short, long)]
    audio: bool,
    /// Highest video quality, e.g. 1080, 720, 480.
    #[arg(short, long, value_name = "HEIGHT")]
    quality: Option<u32>,
    /// MP3 bit rate: 320, 256, 192, 128 (0: best).
    #[arg(short, long, default_value_t = 0, value_name = "KBPS")]
    kbps: u32,
    /// Connections per file.
    #[arg(short, long, default_value_t = 8)]
    connections: usize,
    /// Downloads at the same time.
    #[arg(short = 'j', long, default_value_t = 3)]
    jobs: usize,
    /// Speed limit, e.g. 500K or 2M (bytes per second).
    #[arg(short, long)]
    limit: Option<String>,
    /// Use the login of this browser (chrome, firefox, safari, edge, brave).
    #[arg(long)]
    cookies: Option<String>,
    /// Download the link as a plain file, even from a video site.
    #[arg(long)]
    file: bool,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Info { url, cookies }) => info(&url, cookies.as_deref()),
        Some(Command::Helpers { action }) => helpers(action),
        None if cli.get.urls.is_empty() => {
            use clap::CommandFactory;
            let _ = Cli::command().print_help();
            Ok(())
        }
        None => get(cli.get),
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// `2M` → 2 MiB per second.
fn parse_rate(s: &str) -> Result<u64> {
    let t = s.trim().to_ascii_uppercase();
    let t = t.trim_end_matches("/S").trim_end_matches('B');
    let (num, mul) = match t.chars().last() {
        Some('K') => (&t[..t.len() - 1], 1024.0),
        Some('M') => (&t[..t.len() - 1], 1024.0 * 1024.0),
        Some('G') => (&t[..t.len() - 1], 1024.0 * 1024.0 * 1024.0),
        _ => (t, 1.0),
    };
    match num.trim().parse::<f64>() {
        Ok(v) if v > 0.0 => Ok((v * mul) as u64),
        _ => bail!("not a speed: {s} (try 500K or 2M)"),
    }
}

fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:.bold} [{bar:30.cyan/blue}] {percent:>3}% {msg}")
        .unwrap()
        .progress_chars("━━─")
}

/// Installs `tool` with a progress bar.
fn install_tool(tool: Tool) -> Result<()> {
    let pb = ProgressBar::new(0).with_style(
        ProgressStyle::with_template("{prefix:.bold} [{bar:30.cyan/blue}] {bytes}/{total_bytes} {msg}").unwrap(),
    );
    pb.set_prefix(format!("Installing {}", tool.name()));
    let stop = AtomicBool::new(false);
    let result = tools::install(
        tool,
        &|done, total| {
            if let Some(t) = total {
                pb.set_length(t);
            }
            pb.set_position(done);
        },
        &stop,
    );
    match result {
        Ok(_) => {
            pb.finish_with_message("done");
            Ok(())
        }
        Err(e) => {
            pb.abandon_with_message("failed");
            Err(e)
        }
    }
}

/// Installs the helpers media downloads need, if missing.
fn ensure_helpers(audio: bool, youtube: bool) -> Result<()> {
    if tools::find(Tool::YtDlp).is_none() {
        eprintln!("yt-dlp is needed to read video sites; installing it…");
        install_tool(Tool::YtDlp)?;
    }
    if tools::find(Tool::Ffmpeg).is_none() {
        eprintln!("FFmpeg is needed to {}; installing it…", if audio { "make MP3s" } else { "join video and sound" });
        if let Err(e) = install_tool(Tool::Ffmpeg) {
            eprintln!("warning: {e:#}; continuing without FFmpeg");
        }
    }
    if youtube && tools::js_runtime().is_none() {
        eprintln!("YouTube needs a JavaScript runtime; installing Deno…");
        if let Err(e) = install_tool(Tool::Deno) {
            eprintln!("warning: {e:#}; continuing without it");
        }
    }
    Ok(())
}

fn get(args: Get) -> Result<()> {
    let dir = args.output.clone().unwrap_or_else(swiftdm::paths::default_download_dir);
    let speed_limit = args.limit.as_deref().map(parse_rate).transpose()?.unwrap_or(0);
    let manager = Manager::open(None);
    manager.set_settings(Settings {
        max_active: args.jobs.max(1),
        connections: args.connections.clamp(1, 32),
        speed_limit,
        cookies_from: args.cookies.clone(),
        auto_cookies: args.cookies.is_none(),
    });
    let options = media::Options {
        mode: if args.audio { Mode::Audio } else { Mode::Video },
        max_height: args.quality,
        audio_kbps: args.kbps,
        cookies_from: None,
    };
    let media_wanted = args.audio || args.quality.is_some();
    let mut needs_media = false;
    let mut youtube = false;
    let mut sources = Vec::new();
    for url in &args.urls {
        if !swiftdm::util::is_link(url) {
            bail!("not a link: {url}");
        }
        let media = !args.file
            && (media_wanted || (!media::looks_like_file(url) && matches!(media::route(url), media::Route::Media)));
        needs_media |= media;
        youtube |= media && swiftdm::util::host(url).contains("youtu");
        sources.push(if media { Source::Media(options.clone()) } else { Source::File });
    }
    if needs_media {
        ensure_helpers(args.audio, youtube)?;
    }

    // Ctrl+C stops the downloads.
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let (i, m) = (interrupted.clone(), manager.clone());
        let _ = ctrlc::set_handler(move || {
            i.store(true, Ordering::Relaxed);
            m.pause_all();
        });
    }
    let multi = MultiProgress::new();
    let mut bars = Vec::new();
    for (url, source) in args.urls.iter().zip(sources) {
        let id = manager.add(url, source, None, &dir, None);
        let pb = multi.add(ProgressBar::new(1000).with_style(bar_style()));
        bars.push((id, pb));
    }
    loop {
        let mut running = false;
        for (id, pb) in &bars {
            let Some(item) = manager.item(*id) else { continue };
            let name: String = item.display_name().chars().take(40).collect();
            pb.set_prefix(name);
            match item.status {
                Status::Queued => pb.set_message("waiting"),
                Status::Active => {
                    running = true;
                    pb.set_position((item.fraction().unwrap_or(0.0) * 1000.0) as u64);
                    let msg = match &item.stage {
                        Some(stage) => format!("{stage}…"),
                        None => {
                            let total = item.total.map(format_size).unwrap_or_else(|| "?".into());
                            let eta = item.eta.map(|e| format!(", {} left", format_eta(e))).unwrap_or_default();
                            format!("{} of {total} at {}{eta}", format_size(item.done), format_speed(item.speed))
                        }
                    };
                    pb.set_message(msg);
                }
                _ => {}
            }
            if item.status == Status::Queued {
                running = true;
            }
        }
        if !running {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = multi.clear();
    let mut failed = 0;
    for (id, _) in &bars {
        let Some(item) = manager.item(*id) else { continue };
        match item.status {
            Status::Done => {
                let path = item.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                let size = item.total.map(|t| format!(" ({})", format_size(t))).unwrap_or_default();
                println!("✓ {path}{size}");
            }
            Status::Failed => {
                failed += 1;
                eprintln!("✗ {} — {}", item.display_name(), item.error.as_deref().unwrap_or("failed"));
            }
            _ => eprintln!("■ {} — stopped", item.display_name()),
        }
    }
    if interrupted.load(Ordering::Relaxed) {
        bail!("stopped");
    }
    if failed > 0 {
        bail!("{failed} of {} downloads failed", bars.len());
    }
    Ok(())
}

fn info(url: &str, cookies: Option<&str>) -> Result<()> {
    if tools::find(Tool::YtDlp).is_none() {
        ensure_helpers(false, false)?;
    }
    let stop = AtomicBool::new(false);
    match media::inspect(url, cookies, &stop) {
        Ok(Found::Single(i)) => {
            println!("{}", i.title);
            if !i.uploader.is_empty() {
                println!("  by {}", i.uploader);
            }
            if let Some(d) = i.duration {
                println!("  length   {}", format_eta(d as u64));
            }
            println!("  site     {}", i.site);
            for (h, size) in i.heights.iter().zip(&i.sizes) {
                let size = size.map(|s| format!("  ~{}", format_size(s))).unwrap_or_default();
                println!("  {h:>5}p{size}");
            }
        }
        Ok(Found::Playlist { title, entries }) => {
            println!("{title} — {} items", entries.len());
            for (n, e) in entries.iter().enumerate() {
                println!("  {:>3}. {}  {}", n + 1, e.title, e.url);
            }
        }
        Err(e) if e.kind == media::ErrorKind::Unsupported => {
            let p = swiftdm::http::probe(url)?;
            println!("{}", p.name);
            println!("  size       {}", p.size.map(format_size).unwrap_or_else(|| "unknown".into()));
            println!("  resumable  {}", if p.resumable { "yes (several connections)" } else { "no" });
            println!("  type       {}", p.content_type);
        }
        Err(e) => bail!("{e}"),
    }
    Ok(())
}

fn parse_tools(names: &[String]) -> Result<Vec<Tool>> {
    if names.is_empty() || names.iter().any(|n| n == "all") {
        return Ok(Tool::ALL.to_vec());
    }
    names
        .iter()
        .map(|n| match n.to_ascii_lowercase().as_str() {
            "yt-dlp" | "ytdlp" | "youtube-dl" => Ok(Tool::YtDlp),
            "ffmpeg" => Ok(Tool::Ffmpeg),
            "deno" => Ok(Tool::Deno),
            _ => bail!("unknown helper: {n} (yt-dlp, ffmpeg, deno)"),
        })
        .collect()
}

fn helpers(action: Option<HelperAction>) -> Result<()> {
    match action {
        None => {
            for tool in Tool::ALL {
                let s = tools::status(tool);
                let state = match (&s.path, &s.version) {
                    (Some(p), v) => format!(
                        "{} {}{}",
                        v.as_deref().unwrap_or("?"),
                        if s.managed { "(SwiftDM's own) " } else { "" },
                        p.display()
                    ),
                    (None, _) => "not installed".into(),
                };
                println!("{:<8} {}", tool.name(), state);
                println!("         {}", tool.purpose());
            }
            println!("\nInstall or update: swiftdm helpers install");
        }
        Some(HelperAction::Install { names }) => {
            for tool in parse_tools(&names)? {
                install_tool(tool)?;
            }
        }
        Some(HelperAction::Remove { names }) => {
            for tool in parse_tools(&names)? {
                tools::uninstall(tool)?;
                println!("Removed {}", tool.name());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(parse_rate("500K").unwrap(), 500 * 1024);
        assert_eq!(parse_rate("2M").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_rate("1.5MB/s").unwrap(), 1572864);
        assert_eq!(parse_rate("1000").unwrap(), 1000);
        assert!(parse_rate("fast").is_err());
    }
}
