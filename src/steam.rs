//! Getting the game's files through the player's own Steam.
//!
//! Steam for Mac won't install a Windows-only game: its Install button does
//! nothing. Its console can still fetch the files the player owns with
//! `download_depot <app> <depot>`, into Steam's own folder. This opens the
//! console with that command on the clipboard, follows Steam's content log to
//! show how far it has got, and knows where the files land, so a copy fetched
//! this way (now or before) is found without asking.
//!
//! Not original behaviour.

use std::path::{Path, PathBuf};

/// The Steam release of Red Alert 2 and Yuri's Revenge, and the depot with
/// the game's files (the other, 2229852, isn't needed).
pub const APP: u32 = 2229850;
pub const DEPOT: u32 = 2229851;

/// The depot's size on disk once downloaded, in megabytes (Steam's log gives
/// the download's, which is smaller).
pub const UNPACKED_MB: u64 = 1960;

/// What to type into Steam's console.
pub fn command() -> String {
    format!("download_depot {APP} {DEPOT}")
}

fn steam_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Steam"))
}

/// Whether Steam is installed.
pub fn installed() -> bool {
    steam_dir().is_some_and(|dir| dir.exists()) || Path::new("/Applications/Steam.app").exists()
}

/// Where `download_depot` puts the files.
pub fn depot_dir() -> Option<PathBuf> {
    Some(steam_dir()?.join(format!("Steam.AppBundle/Steam/Contents/MacOS/steamapps/content/app_{APP}/depot_{DEPOT}")))
}

fn content_log() -> Option<PathBuf> {
    Some(steam_dir()?.join("logs/content_log.txt"))
}

/// Puts the command on the clipboard and opens Steam's console. Returns where
/// the content log had got to, to read only what comes after.
pub fn start() -> Result<u64, String> {
    use std::io::Write;
    let mut copy = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().map_err(|error| error.to_string())?;
    copy.stdin.take().ok_or("no clipboard")?.write_all(command().as_bytes()).map_err(|error| error.to_string())?;
    copy.wait().map_err(|error| error.to_string())?;
    std::process::Command::new("open").arg("steam://open/console").status().map_err(|error| error.to_string())?;
    Ok(content_log().and_then(|log| std::fs::metadata(log).ok()).map_or(0, |meta| meta.len()))
}

/// How a download started at `from` (a place in the content log) is going.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Steam has said it is downloading the depot, and how many megabytes.
    pub started: bool,
    pub total_mb: Option<u64>,
    /// Megabytes in the depot's folder so far.
    pub have_mb: u64,
    pub complete: bool,
    /// Something Steam said that stops it (no licence, not logged in).
    pub failed: Option<String>,
}

pub fn progress(from: u64) -> Progress {
    let mut progress = Progress::default();
    let text = content_log().and_then(|log| std::fs::read(log).ok()).map(|bytes| String::from_utf8_lossy(bytes.get(from as usize..).unwrap_or_default()).into_owned()).unwrap_or_default();
    let depot = format!("depot {DEPOT}");
    for line in text.lines() {
        if line.contains(&format!("Downloading {depot}")) {
            progress.started = true;
            // "Downloading depot 2229851 (492 files, 1333 MB) ..."
            progress.total_mb = line.split(", ").nth(1).and_then(|part| part.split_whitespace().next()?.parse().ok());
        }
        if line.contains("Depot download complete") && line.contains(&format!("depot_{DEPOT}")) {
            progress.complete = true;
        }
        if line.contains(&format!("{DEPOT}")) && (line.contains("failed") || line.contains("No subscription") || line.contains("Access Denied")) {
            progress.failed = Some(line.trim().to_owned());
        }
    }
    progress.have_mb = depot_dir().map_or(0, |dir| size(&dir)) / 1_000_000;
    progress
}

/// Bytes in a folder and everything in it.
fn size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => size(&entry.path()),
            _ => entry.metadata().map_or(0, |meta| meta.len()),
        })
        .sum()
}
