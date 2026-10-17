//! Finding the player's copy of the game's files, and the screen that asks
//! for them.
//!
//! Not original behaviour: the original is installed with its files and
//! never has to look for them. This program carries none of the game's files,
//! so it uses the player's own copy, wherever it is.
//!
//! When the game starts, the folder last used (`[Files] GameFolder` in the
//! settings) is looked over (`files::check`). If all is well the game starts
//! straight away. Otherwise this screen opens, saying what is wrong: on the
//! first run that nothing has been chosen yet (and where the game was found,
//! if it could be); later, that the folder is gone (a drive unplugged, the
//! folder moved), can't be read, or lacks something. The player chooses or
//! drops a folder (the folder above the game's will do), it is looked over
//! and, once it passes, Play remembers it and starts the game. The settings
//! and saved games are kept elsewhere (`home`), so none of this touches them.
//!
//! Holding Option while the game starts (or RA2_SETUP=1) opens this screen
//! whatever the state of the folder, to choose another.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::picking::hover::Hovered;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::ui_widgets::{Activate, Button};
use bevy::window::FileDragAndDrop;

use crate::files::{self, GameFiles, Problem};

/// Why the screen is shown.
#[derive(Clone, Debug)]
pub enum Reason {
    /// No folder has been chosen yet.
    FirstRun,
    /// The player asked to choose again.
    Asked(Option<PathBuf>),
    /// The folder last used no longer passes.
    Lost(PathBuf, Vec<Problem>),
}

/// The game's files, if they are where they were last time and all is well
/// with them; otherwise why not.
pub fn locate() -> Result<GameFiles, Reason> {
    // A folder named for this run is the only one tried: a mistake in it is
    // the developer's to see, not something to ask the player about.
    if std::env::var_os("RA2_DIR").is_some() {
        let dir = files::dev_dir();
        let check = files::check(&dir);
        return check.files.filter(|_| check.problems.is_empty()).ok_or(Reason::Lost(dir, check.problems));
    }
    let kept = crate::net::Settings::read().game_dir;
    if option_held() || std::env::var_os("RA2_SETUP").is_some_and(|value| value != "0") {
        return Err(Reason::Asked(kept));
    }
    let Some(dir) = kept.or_else(|| Some(files::dev_dir()).filter(|dir| dir.exists() && !crate::home::bundled())) else {
        return Err(Reason::FirstRun);
    };
    let check = files::check(&dir);
    for warning in &check.warnings {
        warn!("{}: {warning}", dir.display());
    }
    match check.files {
        Some(files) if check.problems.is_empty() => Ok(files),
        _ => Err(Reason::Lost(dir, check.problems)),
    }
}

/// Whether the Option key is held down.
#[cfg(target_os = "macos")]
fn option_held() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceFlagsState(state: i32) -> u64;
    }
    const COMBINED_SESSION_STATE: i32 = 0;
    const ALTERNATE: u64 = 0x0008_0000;
    // SAFETY: a plain query of the keyboard's state.
    unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) & ALTERNATE != 0 }
}

#[cfg(not(target_os = "macos"))]
fn option_held() -> bool {
    false
}

/// What the player chose on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Play,
    Quit,
}

/// Shows the screen until the player chooses to play or to quit.
pub fn run(reason: Reason) -> Outcome {
    let outcome = Arc::new(Mutex::new(Outcome::Quit));
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("{} - Setup", crate::home::APP_NAME),
                resolution: bevy::window::WindowResolution::new(720, 520),
                resizable: false,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(BACKGROUND))
        .insert_resource(Chosen(outcome.clone()))
        .insert_resource(Screen::new(reason))
        .add_systems(Startup, start)
        .add_systems(Update, (drops, finish_jobs, follow_steam, act, draw, hover).chain())
        .add_observer(on_button)
        .add_plugins(crate::debug::DebugPlugin)
        .run();
    *outcome.lock().unwrap()
}

const BACKGROUND: Color = Color::srgb(0.075, 0.075, 0.085);
const PANEL: Color = Color::srgb(0.12, 0.12, 0.135);
const ACCENT: Color = Color::srgb(0.78, 0.09, 0.07);
const TEXT: Color = Color::srgb(0.93, 0.93, 0.93);
const DIM: Color = Color::srgb(0.62, 0.62, 0.66);
const GOOD: Color = Color::srgb(0.30, 0.78, 0.38);
const WARN: Color = Color::srgb(0.95, 0.70, 0.20);
const BAD: Color = Color::srgb(0.92, 0.30, 0.26);

#[derive(Resource)]
struct Chosen(Arc<Mutex<Outcome>>);

/// Work going on in the background.
enum Job {
    /// Looking round the Mac for the game.
    Detecting(Task<Option<PathBuf>>),
    /// The folder dialog is open.
    Picking(Task<Option<PathBuf>>),
    /// Looking over a folder.
    Checking(Task<Report>),
}

/// A folder looked over.
#[derive(Clone)]
struct Report {
    /// The folder as chosen, and the game's folder found at or below it.
    chosen: PathBuf,
    found: Option<PathBuf>,
    problems: Vec<Problem>,
    warnings: Vec<String>,
}

impl Report {
    fn ok(&self) -> bool {
        self.found.is_some() && self.problems.is_empty()
    }
}

/// Waiting for Steam's console to fetch the files.
struct SteamWait {
    /// Where Steam's content log had got to when it began.
    from: u64,
    progress: crate::steam::Progress,
    /// When it was last looked at, and when the folder last grew.
    looked: f64,
    grew: f64,
    error: Option<String>,
}

#[derive(Resource)]
struct Screen {
    steam: Option<SteamWait>,
    reason: Reason,
    job: Option<Job>,
    report: Option<Report>,
    /// Whether the game was looked for and not found.
    not_detected: bool,
    /// A file is being dragged over the window.
    hovering: bool,
    /// The button just pressed.
    pressed: Option<Action>,
    dirty: bool,
    root: Option<Entity>,
    font: Option<Handle<Font>>,
}

impl Screen {
    fn new(reason: Reason) -> Self {
        Self { steam: None, reason, job: None, report: None, not_detected: false, hovering: false, pressed: None, dirty: true, root: None, font: None }
    }

    fn check(&mut self, chosen: PathBuf) {
        self.job = Some(Job::Checking(AsyncComputeTaskPool::get().spawn(async move { report(chosen) })));
        self.dirty = true;
    }
}

/// Looks over a folder the player chose: the game's folder may be it or
/// somewhere below it.
fn report(chosen: PathBuf) -> Report {
    // A file dropped rather than a folder: the folder it is in.
    let chosen = if chosen.is_file() { chosen.parent().map_or(chosen.clone(), Path::to_path_buf) } else { chosen };
    let found = files::search(&chosen, 8, 20_000);
    let (problems, warnings) = match &found {
        Some(dir) => {
            let check = files::check(dir);
            (check.problems, check.warnings)
        }
        None => {
            let check = files::check(&chosen);
            match check.problems.first() {
                Some(Problem::Gone | Problem::NoPermission) => (check.problems, check.warnings),
                _ => (vec![Problem::Missing("ra2.mix")], Vec::new()),
            }
        }
    };
    Report { chosen, found, problems, warnings }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum Action {
    Choose,
    TryAgain,
    Play,
    Quit,
    /// Fetch the files with the player's Steam (see `steam`), or open its
    /// console again while waiting.
    Steam,
    SteamAgain,
}

#[derive(Component)]
struct Primary;

fn start(mut commands: Commands, mut screen: ResMut<Screen>, mut fonts: ResMut<Assets<Font>>) {
    commands.spawn(Camera2d);
    // The Mac's own lettering, where there is one.
    screen.font = ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf"]
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .map(|bytes| fonts.add(Font::from_bytes(bytes)));
    let root = commands
        .spawn(Node { width: percent(100), height: percent(100), flex_direction: FlexDirection::Column, ..default() })
        .id();
    screen.root = Some(root);
    // RA2_SETUP_STEAM=1 opens on the Steam download page without touching
    // Steam, for looking at it.
    if std::env::var_os("RA2_SETUP_STEAM").is_some() {
        screen.steam = Some(SteamWait { from: u64::MAX, progress: Default::default(), looked: 0.0, grew: 0.0, error: None });
    }
    match screen.reason.clone() {
        Reason::FirstRun => {
            screen.job = Some(Job::Detecting(AsyncComputeTaskPool::get().spawn(async { files::detect() })));
        }
        Reason::Asked(Some(dir)) => screen.check(dir),
        Reason::Asked(None) => {}
        Reason::Lost(dir, problems) => {
            screen.report = Some(Report { chosen: dir.clone(), found: Some(dir), problems, warnings: Vec::new() });
        }
    }
}

/// A folder dropped on the window is taken as chosen.
fn drops(mut events: MessageReader<FileDragAndDrop>, mut screen: ResMut<Screen>) {
    for event in events.read() {
        match event {
            FileDragAndDrop::HoveredFile { .. } => {
                screen.hovering = true;
                screen.dirty = true;
            }
            FileDragAndDrop::HoveredFileCanceled { .. } => {
                screen.hovering = false;
                screen.dirty = true;
            }
            FileDragAndDrop::DroppedFile { path_buf, .. } => {
                screen.hovering = false;
                if !matches!(screen.job, Some(Job::Checking(_))) {
                    screen.check(path_buf.clone());
                }
            }
        }
    }
}

fn finish_jobs(mut screen: ResMut<Screen>) {
    let Some(job) = screen.job.as_mut() else {
        return;
    };
    match job {
        Job::Detecting(task) => {
            if let Some(found) = check_ready(task) {
                screen.job = None;
                match found {
                    Some(dir) => screen.check(dir),
                    None => screen.not_detected = true,
                }
                screen.dirty = true;
            }
        }
        Job::Picking(task) => {
            if let Some(picked) = check_ready(task) {
                screen.job = None;
                screen.dirty = true;
                if let Some(dir) = picked {
                    screen.check(dir);
                }
            }
        }
        Job::Checking(task) => {
            if let Some(report) = check_ready(task) {
                screen.job = None;
                screen.report = Some(report);
                screen.dirty = true;
            }
        }
    }
}

/// Follows a download in Steam, once a second; when Steam says it is done (or
/// the folder has stopped growing for ten seconds and holds the game), the
/// folder is looked over as if chosen.
fn follow_steam(mut screen: ResMut<Screen>, time: Res<Time<Real>>) {
    let now = time.elapsed_secs_f64();
    let Some(wait) = screen.steam.as_mut() else {
        return;
    };
    if now - wait.looked < 1.0 {
        return;
    }
    wait.looked = now;
    let progress = crate::steam::progress(wait.from);
    if progress.have_mb != wait.progress.have_mb {
        wait.grew = now;
    }
    let settled = progress.have_mb > 0 && now - wait.grew > 10.0 && crate::steam::depot_dir().is_some_and(|dir| files::looks_like_game(&dir));
    let done = progress.complete || settled;
    let changed = progress != wait.progress;
    wait.progress = progress;
    if done && screen.job.is_none() {
        screen.steam = None;
        if let Some(dir) = crate::steam::depot_dir() {
            screen.check(dir);
        }
    } else if changed {
        screen.dirty = true;
    }
}

fn on_button(activate: On<Activate>, actions: Query<&Action>, mut screen: ResMut<Screen>) {
    if let Ok(&action) = actions.get(activate.entity) {
        screen.pressed = Some(action);
    }
}

/// Carries out the button pressed, or the main one on Return.
// The folder dialog is opened on the main thread: opened from another it
// waits for the main thread, which is waiting for this system to finish.
fn act(_main_thread: bevy::ecs::system::NonSendMarker, keys: Res<ButtonInput<KeyCode>>, mut screen: ResMut<Screen>, chosen: Res<Chosen>, mut exit: MessageWriter<AppExit>) {
    let action = screen.pressed.take().or_else(|| keys.just_pressed(KeyCode::Enter).then_some(page(&screen).primary).flatten());
    let Some(action) = action else {
        return;
    };
    if screen.job.is_some() && action != Action::Quit {
        return;
    }
    match action {
        Action::Choose => {
            screen.steam = None;
            let start = screen.report.as_ref().map(|report| report.chosen.clone()).filter(|dir| dir.exists());
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose your Red Alert 2 folder");
            if let Some(start) = start {
                dialog = dialog.set_directory(start);
            }
            let picking = dialog.pick_folder();
            screen.job = Some(Job::Picking(AsyncComputeTaskPool::get().spawn(async move { picking.await.map(|handle| handle.path().to_path_buf()) })));
            screen.dirty = true;
        }
        Action::Steam | Action::SteamAgain => {
            let (from, error) = match crate::steam::start() {
                Ok(from) => (screen.steam.as_ref().map_or(from, |wait| wait.from), None),
                Err(error) => (0, Some(format!("Couldn't open Steam: {error}"))),
            };
            screen.steam = Some(SteamWait { from, progress: Default::default(), looked: 0.0, grew: 0.0, error });
            screen.dirty = true;
        }
        Action::TryAgain => {
            if let Some(report) = screen.report.clone() {
                screen.check(report.chosen);
            }
        }
        Action::Play => {
            let Some(dir) = screen.report.as_ref().filter(|report| report.ok()).and_then(|report| report.found.clone()) else {
                return;
            };
            let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
            crate::net::Settings { game_dir: Some(dir), ..crate::net::Settings::read() }.save();
            *chosen.0.lock().unwrap() = Outcome::Play;
            exit.write(AppExit::Success);
        }
        Action::Quit => {
            exit.write(AppExit::Success);
        }
    }
}

fn hover(mut buttons: Query<(&Hovered, &mut BackgroundColor, Has<Primary>), (With<Action>, Changed<Hovered>)>) {
    for (hovered, mut colour, primary) in &mut buttons {
        let base = if primary { ACCENT } else { Color::srgb(0.22, 0.22, 0.25) };
        colour.0 = if hovered.0 { base.lighter(0.08) } else { base };
    }
}

/// The lines of the screen.
struct Page {
    heading: String,
    body: Vec<String>,
    folder: Option<String>,
    rows: Vec<(Color, String)>,
    busy: Option<String>,
    buttons: Vec<Action>,
    primary: Option<Action>,
}

fn page(screen: &Screen) -> Page {
    let busy = match &screen.job {
        Some(Job::Detecting(_)) => Some("Looking for Red Alert 2 on this Mac...".to_owned()),
        Some(Job::Picking(_)) => Some("Choose the folder Red Alert 2 is in.".to_owned()),
        Some(Job::Checking(_)) => Some("Checking the game's files...".to_owned()),
        None => None,
    };
    let saves_kept = "Your saved games and settings are kept safe, whatever you choose.".to_owned();
    let what = "This needs the files from your own copy of Red Alert 2 (the Steam or EA app release, or the original CDs installed on a PC). Choose the folder the game is installed in, or drag it onto this window. The folder above it will do.".to_owned();
    let mut out = Page { heading: String::new(), body: Vec::new(), folder: None, rows: Vec::new(), busy, buttons: vec![Action::Quit], primary: None };
    if let Some(wait) = &screen.steam {
        out.heading = "Downloading with Steam".into();
        out.body.push("Steam's console should now be open (if not, open Steam and choose View > Console, or the Console tab). The command below is already copied: click in the box at the bottom of the console, paste it (Command-V) and press Return.".into());
        out.folder = Some(crate::steam::command());
        out.body.push("Steam then downloads about 1.3 GB (1.8 GB once unpacked) into its own folder. You need to own Red Alert 2 on Steam and be signed in. Leave this window open: it carries on by itself when the download is done.".into());
        let p = &wait.progress;
        out.busy = Some(match (&wait.error, &p.failed) {
            (Some(error), _) => error.clone(),
            (None, Some(failed)) => format!("Steam said: {failed}"),
            _ if p.started || p.have_mb > 0 => format!("Downloading: {} of about {} MB", p.have_mb.min(crate::steam::UNPACKED_MB), crate::steam::UNPACKED_MB),
            _ => "Waiting for Steam to start the download...".into(),
        });
        out.buttons.push(Action::Choose);
        out.primary = Some(Action::SteamAgain);
        return out;
    }
    let steam = crate::steam::installed();
    match &screen.report {
        Some(report) if report.ok() => {
            let found = report.found.as_ref().unwrap();
            out.heading = "Ready to play".into();
            out.body.push(match &screen.reason {
                Reason::FirstRun => "Red Alert 2 was found and everything it needs is there. It will be used from now on.".into(),
                _ => "Everything the game needs is there. This folder will be used from now on.".into(),
            });
            out.folder = Some(found.display().to_string());
            out.rows = rows(report);
            out.buttons.push(Action::Choose);
            out.primary = Some(Action::Play);
        }
        Some(report) => {
            let lost = matches!(&screen.reason, Reason::Lost(dir, _) if Some(dir) == report.found.as_ref());
            let shown = report.found.as_ref().unwrap_or(&report.chosen);
            match report.problems.first() {
                Some(Problem::Gone) if lost => {
                    out.heading = "Red Alert 2's files can't be found".into();
                    out.body.push(match drive(shown) {
                        Some(name) => format!("They were on the drive \"{name}\", which isn't connected. Connect it and choose Try Again, or choose where the files are now."),
                        None => "They have been moved or deleted since the game was last played. Choose where they are now.".into(),
                    });
                    out.body.push(saves_kept);
                    out.buttons.push(Action::TryAgain);
                }
                _ if lost && report.problems.contains(&Problem::Missing("ra2.mix")) => {
                    out.heading = "Red Alert 2's files can't be found".into();
                    out.body.push("They're no longer in this folder; the game may have been uninstalled or moved. Choose where they are now.".into());
                    out.body.push(saves_kept);
                    out.buttons.push(Action::TryAgain);
                }
                Some(Problem::Gone) => {
                    out.heading = "That folder can't be found".into();
                    out.body.push(what);
                }
                Some(Problem::NoPermission) => {
                    out.heading = "macOS won't let the game read that folder".into();
                    out.body.push("Open System Settings > Privacy & Security > Files and Folders and allow \"RA2 Engine\" to read it, then choose Try Again. Or move the game's files to another folder and choose that.".into());
                    out.buttons.push(Action::TryAgain);
                }
                _ if report.found.is_none() => {
                    out.heading = "Red Alert 2 isn't in that folder".into();
                    out.body.push(if files::find(&report.chosen, "ra2md.mix").is_some() {
                        "That looks like Yuri's Revenge only. The game needs Red Alert 2's own files (ra2.mix and the rest), which come with it in the Steam and EA app releases.".into()
                    } else {
                        "No Red Alert 2 files (ra2.mix and the rest) were found in it or in the folders inside it.".into()
                    });
                    out.body.push(what);
                }
                _ => {
                    out.heading = if lost { "Some of Red Alert 2's files are missing".into() } else { "That copy of Red Alert 2 can't be used".into() };
                    out.body.push("The game was found, but these need putting right first. Reinstalling or verifying the game's files (in Steam: Properties > Installed Files > Verify) usually does it.".into());
                    out.rows = rows(report);
                    if lost {
                        out.body.push(saves_kept);
                    }
                    out.buttons.push(Action::TryAgain);
                }
            }
            out.folder = Some(shown.display().to_string());
            if steam {
                out.buttons.push(Action::Steam);
            }
            out.primary = Some(Action::Choose);
        }
        None => {
            out.heading = "Find your Red Alert 2 files".into();
            out.body.push(what);
            if screen.not_detected {
                out.body.push("It wasn't found in any of the usual places, so please choose where it is.".into());
            }
            if steam {
                out.body.push("Own it on Steam? Steam for Mac won't install it, but Download with Steam fetches the files through Steam's console.".into());
                out.buttons.push(Action::Steam);
            }
            out.primary = Some(Action::Choose);
        }
    }
    if screen.hovering {
        out.busy = Some("Drop the folder to use it.".into());
    }
    out
}

/// What was looked at, each with how it went.
fn rows(report: &Report) -> Vec<(Color, String)> {
    let mut rows = Vec::new();
    let missing: Vec<&str> = report.problems.iter().filter_map(|problem| if let Problem::Missing(name) = problem { Some(*name) } else { None }).collect();
    if missing.is_empty() {
        rows.push((GOOD, "Game archives (ra2.mix, language.mix and the others)".into()));
    } else {
        rows.push((BAD, format!("Missing from the folder: {}", missing.join(", "))));
    }
    for problem in &report.problems {
        match problem {
            Problem::Damaged(what) => rows.push((BAD, format!("Couldn't be read, it may be damaged: {what}"))),
            Problem::MissingInside(names) => rows.push((BAD, format!("Missing from the archives: {}", names.join(", ")))),
            _ => {}
        }
    }
    if !report.warnings.iter().any(|warning| warning.starts_with(files::EXE)) {
        rows.push((GOOD, "game.exe (version 1.006)".into()));
    }
    if !report.problems.iter().any(|problem| matches!(problem, Problem::MissingInside(_) | Problem::Damaged(_))) && missing.is_empty() {
        rows.push((GOOD, "Rules, art, sounds and maps".into()));
    }
    rows.extend(report.warnings.iter().map(|warning| (WARN, format!("Will play, but: {warning}"))));
    rows
}

/// The name of the removable drive a path is on.
fn drive(path: &Path) -> Option<String> {
    let mut parts = path.strip_prefix("/Volumes").ok()?.components();
    Some(parts.next()?.as_os_str().to_string_lossy().into_owned())
}

fn label(action: Action) -> &'static str {
    match action {
        Action::Choose => "Choose Folder...",
        Action::TryAgain => "Try Again",
        Action::Play => "Play",
        Action::Quit => "Quit",
        Action::Steam => "Download with Steam",
        Action::SteamAgain => "Open Steam Again",
    }
}

fn draw(mut commands: Commands, mut screen: ResMut<Screen>) {
    if !screen.dirty {
        return;
    }
    screen.dirty = false;
    let Some(root) = screen.root else {
        return;
    };
    let page = page(&screen);
    let font = screen.font.clone();
    let text = |words: &str, size: f32, colour: Color| {
        (
            Text::new(words),
            TextFont { font: font.clone().map(FontSource::from).unwrap_or_default(), font_size: FontSize::Px(size), ..default() },
            TextColor(colour),
        )
    };
    commands.entity(root).despawn_related::<Children>();
    commands.entity(root).with_children(|parent| {
        // The title bar.
        parent
            .spawn((Node { padding: UiRect::axes(px(32), px(18)), flex_direction: FlexDirection::Column, ..default() }, BackgroundColor(ACCENT)))
            .with_children(|bar| {
                bar.spawn(text(&crate::home::APP_NAME.to_uppercase(), 30.0, Color::WHITE));
                bar.spawn(text("Plays your own copy of Red Alert 2", 14.0, Color::srgb(1.0, 0.85, 0.82)));
            });
        // The page.
        parent
            .spawn(Node { flex_grow: 1.0, padding: UiRect::axes(px(32), px(22)), flex_direction: FlexDirection::Column, row_gap: px(12), ..default() })
            .with_children(|body| {
                body.spawn(text(&page.heading, 21.0, TEXT));
                for paragraph in &page.body {
                    body.spawn(text(paragraph, 14.0, DIM));
                }
                if let Some(folder) = &page.folder {
                    body.spawn((Node { padding: UiRect::axes(px(12), px(8)), border_radius: BorderRadius::all(px(6)), ..default() }, BackgroundColor(PANEL)))
                        .with_child(text(folder, 13.0, TEXT));
                }
                for (colour, words) in &page.rows {
                    body.spawn(Node { column_gap: px(10), align_items: AlignItems::Center, ..default() }).with_children(|row| {
                        row.spawn((Node { width: px(9), height: px(9), flex_shrink: 0.0, border_radius: BorderRadius::all(px(5)), ..default() }, BackgroundColor(*colour)));
                        row.spawn(text(words, 13.0, TEXT));
                    });
                }
                if let Some(busy) = &page.busy {
                    body.spawn(text(busy, 14.0, WARN));
                }
            });
        // The buttons.
        parent
            .spawn(Node { padding: UiRect::axes(px(32), px(20)), column_gap: px(10), justify_content: JustifyContent::FlexEnd, ..default() })
            .with_children(|footer| {
                let busy = screen.job.is_some();
                for &action in page.buttons.iter().chain(page.primary.as_ref()) {
                    let primary = Some(action) == page.primary;
                    let mut button = footer.spawn((
                        Node { padding: UiRect::axes(px(18), px(9)), border_radius: BorderRadius::all(px(6)), ..default() },
                        BackgroundColor(if primary { ACCENT } else { Color::srgb(0.22, 0.22, 0.25) }),
                    ));
                    button.with_child(text(label(action), 14.0, if busy { DIM } else { Color::WHITE }));
                    if matches!(action, Action::Quit) {
                        button.entry::<Node>().and_modify(|mut node| node.margin.right = auto());
                    }
                    if !busy || matches!(action, Action::Quit) {
                        button.insert((Button, Hovered::default(), action));
                    }
                    if primary {
                        button.insert(Primary);
                    }
                }
            });
    });
}
