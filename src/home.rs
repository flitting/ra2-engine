//! Where the player's own things are kept: the settings, the saved games and
//! the log.
//!
//! Not original behaviour: the original keeps RA2.INI and its saves in the
//! game's folder. Here they are never kept with the game's files, so that
//! choosing another copy of the game, or losing the one chosen, leaves them as
//! they were.
//!
//! Run from a Mac app bundle they go in `~/Library/Application Support/RA2
//! Engine` (and the log in `~/Library/Logs/RA2 Engine`), since a bundle's working
//! folder is `/` and it may itself sit in a folder the player can't write in.
//! Run any other way (`cargo run`) they go in the working folder, as they
//! always have. RA2_HOME names another folder for both.

use std::path::PathBuf;

/// What the program is called: its window, its app and its folders.
/// Not "Red Alert 2": that is the name of the game it plays, which isn't ours.
pub const APP_NAME: &str = "RA2 Engine";
/// The folder's name under Application Support and Logs.
pub const NAME: &str = APP_NAME;
/// The name the folders had before; moved to the new name on first run.
const OLD_NAME: &str = "RA2 Rust";

/// Whether this program is running from inside a Mac app bundle.
pub fn bundled() -> bool {
    std::env::current_exe().ok().is_some_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// The folder the settings and saves are kept in.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RA2_HOME") {
        return dir.into();
    }
    match std::env::var_os("HOME") {
        Some(home) if bundled() => {
            let support = PathBuf::from(home).join("Library/Application Support");
            let (old, new) = (support.join(OLD_NAME), support.join(NAME));
            if old.is_dir() && !new.exists() {
                let _ = std::fs::rename(&old, &new);
            }
            new
        }
        _ => PathBuf::new(),
    }
}

/// Run from a bundle, there is no terminal to write to: what would have been
/// written there goes to `~/Library/Logs/RA2 Engine/ra2.log` instead (the run
/// before kept as `ra2.previous.log`), panics included. Logs is where the
/// Console app looks.
pub fn log_to_file() {
    if !bundled() || std::env::var_os("RA2_HOME").is_some() {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dir = PathBuf::from(home).join("Library/Logs").join(NAME);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let log = dir.join("ra2.log");
    let _ = std::fs::rename(&log, dir.join("ra2.previous.log"));
    let Ok(file) = std::fs::File::create(&log) else {
        return;
    };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: both are open descriptors; the file's descriptor stays open
        // for the life of the process because it is forgotten below.
        unsafe {
            libc::dup2(file.as_raw_fd(), 1);
            libc::dup2(file.as_raw_fd(), 2);
        }
        std::mem::forget(file);
        // No colour codes in a file. SAFETY: nothing else runs yet.
        unsafe { std::env::set_var("NO_COLOR", "1") };
    }
}
