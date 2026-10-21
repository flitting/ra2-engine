//! Saving a game to a file and loading it again.
//!
//! Not original behaviour: the file format. The original writes a Windows
//! compound file of its objects; ours is
//! a header, the match's setup, and the simulation's snapshot
//! (`ra2_sim::World::snapshot`). The rules the player sees are meant to be the
//! original's.
//!
//! The lists of saves are shown by the pause menu (`pause.rs`).
//!

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::game::{GameSetup, MatchSetup};
use crate::shell::Screen;
use crate::units::Sim;

const MAGIC: &[u8; 8] = b"RA2RSAV1";
/// Bumped whenever the header or anything in it changes shape. (The
/// snapshot carries its own number.)
const FORMAT: u32 = 12;

/// Where saves are kept: never in the game's own folder (see `home`).
pub fn folder() -> PathBuf {
    std::env::var_os("RA2_SAVES").map_or_else(|| crate::home::data_dir().join("saves"), PathBuf::from)
}

#[derive(Serialize, Deserialize)]
pub struct SaveFile {
    format: u32,
    /// Kept immediately after format so the list can read just the header.
    pub description: String,
    pub edition: crate::files::Edition,
    pub campaign: Option<crate::campaign::CampaignRun>,
    pub tick: u64,
    /// A digest of the rules and the map it was saved against.
    digest: u64,
    pub setup: MatchSetup,
    /// The house the player sat at, and where the view was.
    pub local: u8,
    pub camera: (f32, f32),
    pub camera_pan: Option<crate::campaign::CampaignCameraPan>,
    pub campaign_audio: Option<crate::audio::CampaignAudioState>,
    pub radar_events: Vec<crate::radar::RadarEvent>,
    /// The simulation.
    pub state: Vec<u8>,
    /// The game from its start, for playing it through again.
    pub recording: Option<Recording>,
}

/// A game from its start: the simulation as it began, and every order
/// carried out since, with the tick it was carried out at. The computer
/// players and all chance are inside the simulation, so these play the game
/// through again exactly (`replay`).
/// Not original behaviour: the original keeps no such record.
#[derive(Serialize, Deserialize, Clone, Default, Resource)]
pub struct Recording {
    pub start: Vec<u8>,
    pub start_tick: u64,
    pub orders: Vec<(u64, ra2_sim::Order)>,
    /// The campaign's external audio/clock observations can change its saved
    /// endgame state while the logic frame is stopped. They are replay inputs
    /// alongside orders, never invented extra simulation frames.
    pub presentation: Vec<MissionPresentationStep>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MissionPresentationStep {
    pub tick: u64,
    pub input: MissionPresentationInput,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum MissionPresentationInput {
    OutcomeWait { elapsed_units: u32, audio_busy: bool },
    DrainMissionOutputs { count: usize },
    DrainCombatEvents { count: usize },
}

#[derive(Default)]
pub(crate) struct ReplayPosition {
    next_order: usize,
    next_presentation: usize,
}

impl ReplayPosition {
    /// `replay` has already applied presentation observations at the returned
    /// frame; orders at that frame are applied only before its next logic step.
    pub(crate) fn after_tick(recording: &Recording, tick: u64) -> Self {
        Self {
            next_order: recording.orders.partition_point(|(at, _)| *at < tick),
            next_presentation: recording.presentation.partition_point(|step| step.tick <= tick),
        }
    }
}

pub(crate) fn replay_advance(world: &mut ra2_sim::World, recording: &Recording, position: &mut ReplayPosition, to: u64) -> Result<(), String> {
    loop {
        let tick = world.current_tick();
        while let Some(step) = recording.presentation.get(position.next_presentation).filter(|step| step.tick <= tick) {
            if step.tick != tick { return Err(format!("recorded presentation frame {} was skipped at tick {tick}", step.tick)); }
            match step.input {
                MissionPresentationInput::OutcomeWait { elapsed_units, audio_busy } => {
                    if !world.mission_presentation_waiting() { return Err(format!("recorded campaign presentation has no matching wait at tick {tick}")); }
                    world.advance_mission_presentation_time(elapsed_units, audio_busy);
                }
                MissionPresentationInput::DrainMissionOutputs { count } => {
                    let actual = world.take_mission_outputs().len();
                    if actual != count { return Err(format!("mission output drain at tick {tick}: expected {count}, found {actual}")); }
                }
                MissionPresentationInput::DrainCombatEvents { count } => {
                    let actual = world.take_events().len();
                    if actual != count { return Err(format!("combat event drain at tick {tick}: expected {count}, found {actual}")); }
                }
            }
            position.next_presentation += 1;
        }
        if tick >= to { return Ok(()); }
        while let Some((_, order)) = recording.orders.get(position.next_order).filter(|(at, _)| *at <= tick) {
            world.order(*order);
            position.next_order += 1;
        }
        world.tick();
        if world.current_tick() == tick && !recording.presentation.get(position.next_presentation).is_some_and(|step| step.tick == tick) {
            return Err(format!("replay stopped at tick {tick} before requested tick {to}; no recorded presentation input can advance it"));
        }
    }
}

/// Plays a saved game's recording through, headless, up to a tick (or as far
/// as the save if `to` is past it). For finding out how a game came to be as
/// it was saved.
pub fn replay(files: &crate::files::GameFiles, save: &SaveFile, to: u64) -> Result<ra2_sim::World, String> {
    if save.edition != files.edition() { return Err("save edition differs from mounted game files".into()); }
    let recording = save.recording.as_ref().ok_or("the save has no recording")?;
    let mut world = match &save.campaign {
        Some(run) => crate::campaign_world::world(files, run)?,
        None => crate::game::match_world(files, &save.setup).ok_or("the map could not be set up")?,
    };
    world.restore(&recording.start)?;
    let to = to.min(save.tick);
    replay_advance(&mut world, recording, &mut ReplayPosition::default(), to)?;
    Ok(world)
}

pub fn write(path: &Path, save: &SaveFile) -> Result<(), String> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend(bincode::serialize(save).map_err(|error| error.to_string())?);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

pub fn read(path: &Path) -> Result<SaveFile, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let body = bytes.strip_prefix(MAGIC).ok_or("not a saved game")?;
    let version = body.get(..4).and_then(|bytes| bytes.try_into().ok()).map(u32::from_le_bytes).ok_or("truncated save header")?;
    if version != FORMAT { return Err(format!("saved by another version (format {version})")); }
    let save: SaveFile = bincode::deserialize(body).map_err(|error| error.to_string())?;
    if save.format != FORMAT {
        return Err(format!("saved by another version (format {})", save.format));
    }
    Ok(save)
}

/// A save as the lists show it.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub description: String,
    pub modified: std::time::SystemTime,
}

/// The saves there are, newest first.
// Every save in the folder
// by the file's modified time, newest first; no fixed slots; one written by
// another version is not listed at all.
pub fn list() -> Vec<Entry> {
    use std::io::Read;
    let mut found: Vec<Entry> = std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("sav")))
        .filter_map(|entry| {
            // Only the front of the file is read: the format and what it is called.
            let mut reader = std::io::BufReader::new(std::fs::File::open(entry.path()).ok()?);
            let mut magic = [0u8; 8];
            reader.read_exact(&mut magic).ok()?;
            let (format, description): (u32, String) = bincode::deserialize_from(&mut reader).ok()?;
            // A save whose simulation is of another shape cannot be loaded;
            // it is left out. Its number is the first four bytes of the state.
            let current = (&magic == MAGIC && format == FORMAT) && read(&entry.path()).ok().is_some_and(|save| save.state.get(..4) == Some(&ra2_sim::snapshot::FORMAT.to_le_bytes()[..]));
            current.then(|| Entry { path: entry.path(), description, modified: entry.metadata().and_then(|data| data.modified()).unwrap_or(std::time::UNIX_EPOCH) })
        })
        .collect();
    found.sort_by(|a, b| b.modified.cmp(&a.modified));
    found
}

/// A date and time for the list.
pub fn stamp(time: std::time::SystemTime) -> String {
    let seconds = time.duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs()) as i64;
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days to a date in the civil calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (day, month) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}  {:02}:{:02} UTC", rest / 3600, rest % 3600 / 60)
}

/// The rules and map a game is played against, boiled down to a number, so
/// that a save is not loaded against others.
fn digest(files: &crate::files::GameFiles, setup: &MatchSetup, campaign: Option<&crate::campaign::CampaignRun>) -> u64 {
    let map = [format!("{}.map", setup.map), format!("{}.mpr", setup.map)];
    let names = ["rules.ini", "art.ini", "ai.ini", setup.mode_rules.as_str(), map[0].as_str(), map[1].as_str()];
    let mut hash = names.iter().filter_map(|name| files.get(&name.to_lowercase())).flatten().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ *byte as u64).wrapping_mul(0x100_0000_01b3));
    let identity = bincode::serialize(&(files.edition(), campaign)).expect("serializing session identity");
    for byte in identity { hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3); }
    if campaign.is_some() {
        for byte in ["battle.ini", "mission.ini", "mapsel.ini"].iter().filter_map(|name| files.get(name)).flatten() {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
        }
    }
    hash
}

/// Which of the pause menu's lists is up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Load,
    Save,
    Delete,
}

/// What has been asked for and not yet done, and the list the pause menu shows.
#[derive(Resource, Default)]
pub struct SaveLoad {
    pub save: Option<PathBuf>,
    pub load: Option<PathBuf>,
    /// What the next save is to be called.
    pub description: String,
    /// The list: the saves, the row picked, what has been typed as a
    /// description, and whether it needs drawing again.
    pub entries: Vec<Entry>,
    pub picked: usize,
    pub text: String,
    pub dirty: bool,
    /// A save asked for from the list is being written.
    pub saving: bool,
    /// The row last clicked and when, to tell a double click.
    pub clicked: Option<(usize, f64)>,
    /// A question or notice over the list, waiting for its answer.
    pub ask: Option<Ask>,
}

/// What a box over a list of saves is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Saving needs a description.
    MustDescribe,
    /// Write over this save?
    Overwrite(PathBuf),
    /// Delete this save?
    Delete(PathBuf, String),
    /// The game was saved.
    Saved,
}

/// The name of a new save.
// `SAVE` and four hex
// digits at random, tried again until no such file exists.
pub fn new_name() -> PathBuf {
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |since| since.as_nanos() as u64);
    loop {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let path = folder().join(format!("SAVE{:04X}.SAV", (seed >> 33) & 0x7fff));
        if !path.exists() {
            return path;
        }
    }
}

/// What a save is called when nothing else is typed: the scenario's name.
pub fn default_description(setup: &MatchSetup) -> String {
    if setup.map_name.is_empty() { setup.map.clone() } else { setup.map_name.clone() }
}

/// A save read from disk, waiting for the game it belongs to to be set up.
#[derive(Resource)]
pub struct PendingLoad(pub SaveFile);

pub struct SavesPlugin;

impl Plugin for SavesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SaveLoad>()
            .add_systems(Startup, |mut asked: ResMut<SaveLoad>| {
                // RA2_LOAD=path starts by loading that save.
                asked.load = std::env::var_os("RA2_LOAD").map(PathBuf::from);
            })
            .add_systems(Update, (save.run_if(resource_exists::<Sim>).after(crate::game::CameraScrollSet).after(crate::audio::CampaignAudioSet).after(crate::radar::RadarPresentationSet).after(crate::campaign::CampaignOutcomeSet), load, type_description.run_if(resource_exists::<Sim>)))
            // A game is loaded over a running one by leaving it and coming
            // straight back.
            .add_systems(OnEnter(Screen::Loading), |mut next: ResMut<NextState<Screen>>| next.set(Screen::InGame));
    }
}

fn save(
    mut asked: ResMut<SaveLoad>,
    sim: Res<Sim>,
    recording: Option<Res<Recording>>,
    setup: Option<Res<MatchSetup>>,
    campaign: Option<Res<crate::campaign::ActiveCampaign>>,
    camera_pan: Option<Res<crate::campaign::CampaignCameraPan>>,
    campaign_audio: Option<Res<crate::audio::CampaignAudioState>>,
    radar_events: Res<crate::radar::RadarEvents>,
    camera: Single<&Transform, With<Camera2d>>,
    files: Res<crate::files::GameFiles>,
    mut status: MessageWriter<crate::bar::Status>,
    mut at_tick: Local<Option<(u64, PathBuf)>>,
) {
    // RA2_SAVE_AT=tick,path saves at a tick, for trying this without a mouse.
    if at_tick.is_none() {
        let spec = std::env::var("RA2_SAVE_AT").unwrap_or_default();
        *at_tick = Some(match spec.split_once(',') {
            Some((tick, path)) => (tick.trim().parse().unwrap_or(u64::MAX), PathBuf::from(path.trim())),
            None => (u64::MAX, PathBuf::new()),
        });
    }
    if let Some((tick, path)) = at_tick.as_mut().filter(|(tick, _)| sim.0.current_tick() >= *tick) {
        *tick = u64::MAX;
        asked.save = Some(path.clone());
    }
    let (Some(path), Some(setup)) = (asked.save.take(), setup) else {
        return;
    };
    let file = SaveFile {
        format: FORMAT,
        edition: files.edition(),
        campaign: campaign.as_ref().map(|campaign| campaign.0.clone()),
        // With nothing
        // typed, the scenario's name.
        description: Some(std::mem::take(&mut asked.description)).filter(|text| !text.trim().is_empty()).unwrap_or_else(|| default_description(&setup)),
        tick: sim.0.current_tick(),
        digest: digest(&files, &setup, campaign.as_ref().map(|campaign| &campaign.0)),
        setup: setup.clone(),
        local: crate::terrain::local(),
        camera: (camera.translation.x, camera.translation.y),
        camera_pan: camera_pan.as_deref().cloned(),
        campaign_audio: campaign_audio.as_deref().cloned(),
        radar_events: radar_events.0.clone(),
        state: sim.0.snapshot(),
        // The orders since the game was set up (or loaded) follow those the
        // recording already held.
        recording: recording.map(|recording| Recording { orders: recording.orders.iter().chain(sim.2.iter()).copied().collect(), ..recording.clone() }),
    };
    // Afterwards the
    // player is told the game was saved (`TXT_GAME_WAS_SAVED`).
    match write(&path, &file) {
        Ok(()) => {
            info!("saved {} at tick {} ({} bytes of state)", path.display(), file.tick, file.state.len());
            status.write(crate::bar::Status::Key("TXT_GAME_WAS_SAVED"));
        }
        Err(error) => {
            error!("could not save {}: {error}", path.display());
            status.write(crate::bar::Status::Text(format!("Could not save: {error}")));
        }
    }
}

fn load(mut commands: Commands, mut asked: ResMut<SaveLoad>, mut setup: Option<ResMut<GameSetup>>, state: Res<State<Screen>>, mut next: ResMut<NextState<Screen>>, files: Res<crate::files::GameFiles>, mut status: MessageWriter<crate::bar::Status>, mut notices: MessageWriter<crate::notice::Notice>) {
    let Some(setup) = setup.as_mut() else {
        return;
    };
    let Some(path) = asked.load.take() else {
        return;
    };
    match read(&path) {
        Ok(file) if file.edition != files.edition() => {
            notices.write(crate::notice::Notice("That saved game belongs to a different game edition.".into()));
        }
        // Saved against other rules or another map file: the numbers in it
        // would mean other things.
        Ok(file) if file.digest != digest(&files, &file.setup, file.campaign.as_ref()) => {
            error!("{} was saved with different game files", path.display());
            status.write(crate::bar::Status::Text("That game was saved with different game files".to_owned()));
            notices.write(crate::notice::Notice("That game was saved with different game files.".to_owned()));
        }
        Ok(file) => {
            info!("loading {} (tick {})", path.display(), file.tick);
            // The match is set up again exactly as it was, every choice that
            // was left to chance already settled; then the saved state is
            // laid over it (`game::load_map`).
            **setup = file.setup.as_game_setup();
            commands.remove_resource::<crate::campaign::PendingCampaign>();
            commands.insert_resource(PendingLoad(file));
            next.set(if *state.get() == Screen::InGame { Screen::Loading } else { Screen::InGame });
        }
        Err(error) => {
            error!("could not load {}: {error}", path.display());
            notices.write(crate::notice::Notice("That saved game could not be loaded.".to_owned()));
        }
    }
}

/// Typing while the Save list is up goes to the description.
fn type_description(mut keys: MessageReader<bevy::input::keyboard::KeyboardInput>, mut asked: ResMut<SaveLoad>, menu: Res<crate::pause::Menu>) {
    use bevy::input::keyboard::Key;
    let saving = *menu == crate::pause::Menu::Saves(Kind::Save) && asked.ask.is_none();
    for event in keys.read() {
        if !saving || !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => {
                asked.text.pop();
            }
            // At most 79 characters.
            Key::Space if asked.text.len() < 79 => asked.text.push(' '),
            Key::Character(typed) => {
                for c in typed.chars().filter(|c| c.is_ascii_graphic()) {
                    if asked.text.len() < 79 {
                        asked.text.push(c);
                    }
                }
            }
            _ => continue,
        }
        asked.dirty = true;
    }
}

/// Optional fixtures for checking the real app's endgame bridge. The map and
/// rules are original, but the pending result is injected: this does not prove
/// that the mission's actual objectives can be completed.
#[cfg(test)]
pub(crate) fn export_outcome_fixtures(files: &crate::files::GameFiles, run: &crate::campaign::CampaignRun, world: &mut ra2_sim::World, dir: &Path) {
    let baseline = world.snapshot();
    for won in [true, false] {
        world.restore(&baseline).unwrap();
        let tick = world.current_tick();
        let state = world.mission_state.as_mut().unwrap();
        state.outcome = None;
        state.endgame = ra2_sim::mission::MissionEndgame {
            winner: won, loser: !won,
            timer: ra2_sim::mission::OutcomeTimer { start_tick: tick as i32, length_ticks: 90 },
            presentation_managed: true, ..Default::default()
        };
        state.outputs.push(ra2_sim::mission::MissionOutput::Text { label: if won { "TXT_SCENARIO_WON" } else { "TXT_SCENARIO_LOST" }.into() });
        state.outputs.push(ra2_sim::mission::MissionOutput::Speech { name: if won { "EVA_MissionAccomplished" } else { "EVA_MissionFailed" }.into() });
        world.over = None;
        let setup = MatchSetup { map: run.map_stem().to_owned(), map_name: run.scenario.clone(), options: crate::game::Options::default(), max_players: 1, mode_rules: String::new() };
        let state = world.snapshot();
        let file = SaveFile {
            format: FORMAT, description: format!("Injected pending {}", if won { "victory" } else { "defeat" }),
            edition: run.edition, campaign: Some(run.clone()), tick,
            digest: digest(files, &setup, Some(run)), setup,
            local: world.mission_data.as_ref().unwrap().player_house, camera: (0.0, 0.0),
            camera_pan: None, campaign_audio: None, radar_events: Vec::new(),
            recording: Some(Recording { start: state.clone(), start_tick: tick, ..Recording::default() }), state,
        };
        write(&dir.join(format!("{:?}-{}-{}.ra2save", run.edition, run.scenario, if won { "win" } else { "lose" })), &file).unwrap();
    }
    world.restore(&baseline).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_file_comes_back_as_it_was_written() {
        let setup = MatchSetup { map: "mp03t4".into(), map_name: "Anytown".into(), options: crate::game::Options::default(), max_players: 4, mode_rules: "MPBattle.ini".into() };
        let mut file = SaveFile { format: FORMAT, edition: crate::files::Edition::RedAlert2, campaign: None, description: "a game".into(), tick: 1234, digest: 7, setup, local: 1, camera: (10.0, -20.0), camera_pan: None, campaign_audio: None, radar_events: Vec::new(), state: (0..=255).collect(), recording: None };
        let path = std::env::temp_dir().join(format!("ra2-rust-save-test-{}.sav", std::process::id()));
        write(&path, &file).unwrap();
        let back = read(&path).unwrap();
        assert_eq!((back.tick, back.local, back.camera, &back.state, back.setup.map.as_str()), (1234, 1, (10.0, -20.0), &file.state, "mp03t4"));
        // The list reads only the front of it.
        let front: (u32, String) = bincode::deserialize(&std::fs::read(&path).unwrap()[8..]).unwrap();
        assert_eq!(front, (FORMAT, "a game".to_owned()));
        // A save made during a scripted camera move retains both the session
        // identity and interpolation state, rather than resuming at its end.
        file.edition = crate::files::Edition::YuriAssets;
        file.campaign = Some(crate::campaign::CampaignRun { edition: file.edition, side: crate::campaign::CampaignSide::Soviet, stage: 0, scenario: "sov01umd.map".into(), difficulty: crate::campaign::CampaignDifficulty::Hard });
        file.camera_pan = Some(crate::campaign::CampaignCameraPan { start: [10.0, 20.0], target: [100.0, 200.0], speed_index: 2, fraction: 0.375, last_logic_tick: 1234 });
        file.campaign_audio = Some(crate::audio::CampaignAudioState { current_line: Some("EVA_NewObjectiveReceived".into()), ..Default::default() });
        let mut radar = crate::radar::RadarEvents::default();
        radar.raise(2, (22, 47), file.tick);
        file.radar_events = radar.0;
        file.recording = Some(Recording { presentation: vec![MissionPresentationStep { tick: 1234,
            input: MissionPresentationInput::OutcomeWait { elapsed_units: 17, audio_busy: true } }], ..Recording::default() });
        write(&path, &file).unwrap();
        let campaign = read(&path).unwrap();
        assert_eq!(campaign.edition, file.edition);
        assert_eq!(campaign.campaign, file.campaign);
        assert_eq!(campaign.camera_pan, file.camera_pan);
        assert_eq!(campaign.campaign_audio, file.campaign_audio);
        assert_eq!(campaign.radar_events, file.radar_events);
        assert_eq!(campaign.recording.unwrap().presentation, file.recording.as_ref().unwrap().presentation);
        file.format = FORMAT - 1;
        write(&path, &file).unwrap();
        assert!(read(&path).err().unwrap().contains("saved by another version"));
        assert!(stamp(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_791_878_400)).starts_with("2026-10-13"));
        // Anything else is refused.
        std::fs::write(&path, b"not a save").unwrap();
        assert!(read(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn campaign_recording_replays_output_drains_and_wall_wait_at_one_logic_frame() {
        use ra2_sim::{House, Map, World, mission::*};
        for yuri in [false, true] {
            let mut world = World::new(Map::new(16, 16), None);
            world.houses = vec![House::default(), House { country: 1, human: true, ..House::default() }];
            world.attach_mission(MissionData {
                id: "presentation-replay".into(), yuri, player_house: 1, savour_delay_ticks: 3,
                houses: vec![HouseBinding { name: "Player House".into(), country: "Player".into(), house: 1 }],
                country_names: vec!["Neutral".into(), "Player".into()],
                tags: vec![TagDef { id: "result-tag".into(), trigger: "result".into(), ..TagDef::default() }],
                triggers: vec![TriggerDef { id: "result".into(), country: "Player".into(),
                    events: vec![EventDef { kind: 8, ..EventDef::default() }],
                    actions: vec![ActionDef::with_parameter(1, "1")], ..TriggerDef::default() }],
                ..MissionData::default()
            }).unwrap();
            world.set_mission_presentation_managed(true);
            let mut recording = Recording { start: world.snapshot(), start_tick: world.current_tick(), ..Recording::default() };
            for _ in 0..30 {
                world.tick();
                let count = world.take_mission_outputs().len();
                if count > 0 { recording.presentation.push(MissionPresentationStep { tick: world.current_tick(), input: MissionPresentationInput::DrainMissionOutputs { count } }); }
                if world.mission_presentation_waiting() { break; }
            }
            assert!(world.mission_presentation_waiting());
            let waiting_tick = world.current_tick();
            for (elapsed_units, audio_busy) in [(0, true), (17, true), (9, true), (0, false)] {
                recording.presentation.push(MissionPresentationStep { tick: waiting_tick, input: MissionPresentationInput::OutcomeWait { elapsed_units, audio_busy } });
                world.advance_mission_presentation_time(elapsed_units, audio_busy);
                let expected = world.snapshot();
                world.restore(&recording.start).unwrap();
                replay_advance(&mut world, &recording, &mut ReplayPosition::default(), waiting_tick).unwrap();
                assert_eq!(world.snapshot(), expected, "same-frame audio wait diverged for YR={yuri}");
            }
            assert!(world.over.is_some());
            let count = world.take_mission_outputs().len();
            assert_eq!(count, 1);
            recording.presentation.push(MissionPresentationStep { tick: waiting_tick, input: MissionPresentationInput::DrainMissionOutputs { count } });
            let expected = world.snapshot();
            world.restore(&recording.start).unwrap();
            let mut position = ReplayPosition::default();
            replay_advance(&mut world, &recording, &mut position, 2).unwrap();
            position = ReplayPosition::after_tick(&recording, world.current_tick());
            replay_advance(&mut world, &recording, &mut position, waiting_tick).unwrap();
            assert_eq!(world.snapshot(), expected);
            let error = replay_advance(&mut world, &recording, &mut position, waiting_tick + 1).unwrap_err();
            assert!(error.contains("replay stopped"));
            assert_eq!(world.current_tick(), waiting_tick);
        }
    }
}

