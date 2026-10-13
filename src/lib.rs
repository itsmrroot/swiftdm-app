//! SwiftDM — download videos, music and files from any link.
//!
//! The engine behind the `swiftdm` command line and the desktop app:
//! fast resumable file downloads over several connections ([`http`]),
//! videos and music from 1800+ sites through yt-dlp ([`media`]), the helper
//! programs that needs ([`tools`]), and the download list ([`manager`]).
//!
//! Powered by Bashar Salmo.

pub mod http;
pub mod manager;
pub mod media;
pub mod paths;
pub mod tools;
pub mod util;
