//! Sound effects, the announcer and music, from the game's own files.
//!
//! DATA: `sound.ini` names the samples and volume of each sound, `eva.ini` the
//! announcer's lines per side, `theme.ini` the music tracks; `RA2.INI [Audio]`
//! holds the player's three volume settings.
//!
//! The rules for sounds and the announcer follow the original game.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bevy::audio::Volume;
use bevy::prelude::*;
use ra2_formats::audio::{AudioIndex, Pcm};
use ra2_formats::ini::Ini;
use serde::{Deserialize, Serialize};

use crate::files::GameFiles;
use crate::shell::Screen;

/// Something to be heard.
#[derive(Message, Clone, Debug)]
pub enum Cue {
    /// A `sound.ini` section, such as a unit's `VoiceSelect`.
    Sound(String),
    /// The same at a share of its volume.
    SoundAt(String, f32),
    /// A sound made at a point on the map (scene pixels): how loud it is depends
    /// on how far that is from the view.
    SoundOn(String, Vec2),
    /// An `eva.ini` section, such as `EVA_ConstructionComplete`.
    Eva(&'static str),
    /// A mission-selected EVA/dialog section, owned for the duration of its
    /// queue entry rather than leaked into a process-lifetime string.
    Speech(String),
}

/// Presentation state already consumed from the simulation's mission output
/// queue. Saving only that queue loses speech which is waiting or playing here.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignAudioState {
    pub waiting: Vec<CampaignSpeechEntry>,
    /// Native current-line deduplication lasts through the post-speech gap.
    pub current_line: Option<String>,
    pub playing: Option<CampaignSpeechPlayback>,
    /// Some(0) still waits for a later clock value (native strict `>` gate).
    pub gap_remaining: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeechKind {
    #[default]
    Standard,
    Queue,
    Interrupt,
    QueuedInterrupt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignSpeechEntry {
    pub line: String,
    pub priority: u8,
    pub kind: SpeechKind,
}

impl CampaignSpeechEntry {
    fn rank(&self) -> u8 {
        match (self.kind, self.priority) {
            (SpeechKind::QueuedInterrupt, _) => 0,
            (SpeechKind::Queue, priority) => 6 - priority.min(3),
            (_, 3) => 1,
            _ => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignSpeechPlayback {
    pub line: String,
    /// The selected voice sample, retaining its original announcer on load.
    pub sample: String,
    pub sample_rate: u32,
    /// PCM frames from the start of that sample, shared by all its channels.
    pub frame_offset: u64,
    pub paused: bool,
}

/// Insert after a successful world restore. The audio system stops the old
/// voice before restoring the saved queue and selected sample.
#[derive(Resource)]
pub struct PendingCampaignAudio(pub CampaignAudioState);

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CampaignAudioSet;

/// An `[AudioVisual]` or `[General]` key of rules.ini that names a sound.
pub fn rules_sound(files: &GameFiles, key: &str) -> Option<String> {
    let rules = Ini::parse(files.get("rules.ini")?);
    ["AudioVisual", "General"]
        .iter()
        .find_map(|section| rules.get(section, key))
        .and_then(|value| value.split(',').next())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// Every name in an `[AudioVisual]` (or `[General]`) list in rules.ini.
pub fn rules_list(files: &GameFiles, key: &str) -> Vec<String> {
    let Some(rules) = files.get("rules.ini").map(Ini::parse) else {
        return Vec::new();
    };
    ["AudioVisual", "General"]
        .iter()
        .find_map(|section| rules.get(section, key))
        .map(|value| value.split(',').map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()).collect())
        .unwrap_or_default()
}

fn announcer_key(side: i32) -> &'static str {
    match side { 1 => "Russian", 2 => "Yuri", _ => "Allied" }
}

#[derive(Resource)]
pub(crate) struct Audio {
    indices: Vec<Option<AudioIndex>>,
    sounds: Option<Ini>,
    eva: Option<Ini>,
    /// Side of each country, retaining the rule file's country indices.
    announcer_sides: Vec<i32>,
    themes: Option<Ini>,
    /// `GUIMainButtonSound`.
    button_sound: Option<String>,
    sound_volume: f32,
    voice_volume: f32,
    score_volume: f32,
    decoded: HashMap<String, (Handle<AudioSource>, u32)>,
    /// Advances with each random pick.
    seed: u32,
    /// Position in the in-game play list.
    track: usize,
    /// Sounds asked for since the last service pass, and when that was.
    pending: Vec<(String, f32, Option<Vec2>)>,
    serviced: f64,
    /// Type-specific FIFOs and an ordinary slot, represented in arrival order.
    eva_waiting: Vec<CampaignSpeechEntry>,
    /// Retained after playback until its post-speech delay has elapsed.
    eva_playing: Option<String>,
    eva_ended: Option<u64>,
    eva_delay: u64,
}

/// A sound effect that is playing: its `sound.ini` section and when it started.
#[derive(Component)]
struct Playing(String, f64);

/// The samples still to come after the one an entity is playing.
#[derive(Component)]
struct Chain {
    rest: Vec<Handle<AudioSource>>,
    level: f32,
    speed: f32,
}

/// Starts the next sample of a sound made of several once the last has ended.
fn play_chains(mut commands: Commands, mut done: Query<(Entity, &mut Chain), Without<AudioPlayer>>) {
    for (entity, mut chain) in &mut done {
        if chain.rest.is_empty() {
            commands.entity(entity).despawn();
            continue;
        }
        let next = chain.rest.remove(0);
        let settings = if chain.rest.is_empty() { PlaybackSettings::DESPAWN } else { PlaybackSettings::REMOVE };
        commands.entity(entity).insert((AudioPlayer::new(next), settings.with_volume(Volume::Linear(chain.level)).with_speed(chain.speed)));
        if chain.rest.is_empty() {
            commands.entity(entity).remove::<Chain>();
        }
    }
}

/// The announcer's line that is playing.
#[derive(Component)]
struct Speaking {
    /// A restored source is an exact PCM suffix, so its sink starts at zero.
    start_frame: u64,
    rate: u32,
}

/// Marks the entity playing music, with which it is: 0 the menus', 1 the
/// in-game play list, 2 the score screen's.
#[derive(Component)]
struct Music(u8);

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Cue>()
            .init_resource::<CampaignAudioState>()
            .add_systems(Update, restore_silent_audio.in_set(CampaignAudioSet)
                .after(crate::campaign::CampaignPresentationSet).run_if(not(resource_exists::<Audio>)))
            .add_systems(OnEnter(Screen::LoadScreen), clear_campaign_audio)
            .add_systems(OnEnter(Screen::Loading), clear_campaign_audio);
        // RA2_MUTE=1 turns all sound off, for automated runs.
        if std::env::var_os("RA2_MUTE").is_some() {
            return;
        }
        app.add_systems(Startup, setup)
            .add_systems(Update, (restore_campaign_audio, play_cues).chain().in_set(CampaignAudioSet)
                .after(crate::campaign::CampaignPresentationSet).run_if(resource_exists::<Audio>))
            .add_systems(Update, (follow_prefs, play_chains, play_music).run_if(resource_exists::<Audio>))
            .add_observer(on_button);
    }
}

/// The three volumes follow the Options screen, the music that is playing
/// at once.
// The volume sliders take effect as they move.
fn follow_prefs(prefs: Res<crate::prefs::Prefs>, mut audio: ResMut<Audio>, mut music: Query<&mut AudioSink, With<Music>>) {
    if !prefs.is_changed() {
        return;
    }
    audio.sound_volume = prefs.sound;
    audio.voice_volume = prefs.voice;
    audio.score_volume = prefs.music;
    for mut sink in &mut music {
        sink.set_volume(Volume::Linear(prefs.music));
    }
}

fn setup(mut commands: Commands, files: Res<GameFiles>, prefs: Res<crate::prefs::Prefs>) {
    let ini = |name: &str| files.get(name).map(Ini::parse);
    let volume = |key: &str, _default: f32| match key {
        "SoundVolume" => prefs.sound,
        "VoiceVolume" => prefs.voice,
        _ => prefs.music,
    };
    commands.insert_resource(Audio {
        indices: files.audio_banks().map(|(index, _)| AudioIndex::parse(index)).collect(),
        sounds: ini("sound.ini"),
        eva: ini("eva.ini"),
        announcer_sides: ini("rules.ini").map(|rules| (0..rules.section("Countries").len())
            .map(|country| crate::terrain::country_side(&rules, country)).collect()).unwrap_or_default(),
        themes: ini("theme.ini"),
        button_sound: rules_sound(&files, "GUIMainButtonSound"),
        sound_volume: volume("SoundVolume", 0.7),
        voice_volume: volume("VoiceVolume", 0.8),
        score_volume: volume("ScoreVolume", 0.6),
        decoded: HashMap::new(),
        seed: 0x2545_f491,
        track: 0,
        pending: Vec::new(),
        serviced: 0.0,
        eva_waiting: Vec::new(),
        eva_playing: None,
        eva_ended: None,
        eva_delay: 500,
    });
}

impl Audio {
    fn queue_speech(&mut self, line: &str, now: u64) -> Option<bool> {
        if self.eva_playing.as_deref().is_some_and(|playing| playing.eq_ignore_ascii_case(line))
            || self.eva_waiting.iter().any(|waiting| waiting.line.eq_ignore_ascii_case(line)) { return None }
        let get = |key: &str| self.eva.as_ref().and_then(|eva| eva.get(line, key)).map(|v| v.trim().to_uppercase());
        let priority = match get("Priority").as_deref() { Some("LOW") => 0, Some("IMPORTANT") => 2, Some("CRITICAL") => 3, _ => 1 };
        let kind = match get("Type").as_deref() {
            Some("QUEUE") => SpeechKind::Queue,
            Some("INTERRUPT") => SpeechKind::Interrupt,
            Some("QUEUED_INTERRUPT") => SpeechKind::QueuedInterrupt,
            _ => SpeechKind::Standard,
        };
        let interrupted = kind == SpeechKind::Interrupt && self.eva_playing.is_some();
        if interrupted {
            self.eva_waiting.clear();
            self.eva_playing = None;
            self.eva_ended = Some(now);
            self.eva_delay = 0;
        }
        let entry = CampaignSpeechEntry { line: line.to_owned(), priority, kind };
        if entry.rank() != 2 {
            self.eva_waiting.push(entry);
        } else {
            // Native RA2 0x715a69 / YR 0x752629 rejects ordinary requests
            // while either dedicated high FIFO is nonempty.
            if self.eva_waiting.iter().any(|entry| entry.rank() < 2) { return Some(interrupted) }
            match self.eva_waiting.iter().position(|entry| entry.rank() == 2) {
                None => self.eva_waiting.push(entry),
                Some(at) if self.eva_waiting[at].priority < priority => self.eva_waiting[at] = entry,
                Some(_) => {}
            }
        }
        Some(interrupted)
    }

    fn next_speech(&mut self, now: u64) -> Option<CampaignSpeechEntry> {
        if self.eva_playing.is_some() && self.eva_ended.is_none() { return None }
        if self.eva_ended.is_some_and(|end| now <= end.saturating_add(self.eva_delay)) { return None }
        self.eva_playing = None;
        self.eva_ended = None;
        let at = (0..self.eva_waiting.len()).min_by_key(|&at| self.eva_waiting[at].rank())?;
        let entry = self.eva_waiting.remove(at);
        if entry.rank() < 2 {
            self.eva_waiting.retain(|entry| entry.rank() != 2);
        }
        Some(entry)
    }

    fn random(&mut self, count: usize) -> usize {
        // Not part of the simulation, so any generator will do.
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 16) as usize % count.max(1)
    }

    /// A sample by name: from the sound bank, or else a wave file of that name.
    fn sample(
        &mut self,
        files: &GameFiles,
        sources: &mut Assets<AudioSource>,
        name: &str,
    ) -> Option<Handle<AudioSource>> {
        let name = name.to_lowercase();
        if let Some((handle, _)) = self.decoded.get(&name) {
            return Some(handle.clone());
        }
        let banked = || self.indices.iter().zip(files.audio_banks())
            .find_map(|(index, (_, bag))| index.as_ref()?.decode(&name, bag));
        let pcm = banked().or_else(|| Pcm::from_wav(files.get(&format!("{name}.wav"))?))?;
        let handle = sources.add(AudioSource { bytes: Arc::from(pcm.to_wav()) });
        self.decoded.insert(name, (handle.clone(), pcm.rate));
        Some(handle)
    }

    fn pcm(&self, files: &GameFiles, name: &str) -> Option<Pcm> {
        self.indices.iter().zip(files.audio_banks())
            .find_map(|(index, (_, bag))| index.as_ref()?.decode(name, bag))
            .or_else(|| Pcm::from_wav(files.get(&format!("{name}.wav"))?))
    }
}

fn clear_campaign_audio(mut commands: Commands, audio: Option<ResMut<Audio>>, mut state: ResMut<CampaignAudioState>, speaking: Query<Entity, With<Speaking>>) {
    for entity in &speaking { commands.entity(entity).despawn(); }
    if let Some(mut audio) = audio {
        audio.eva_waiting.clear();
        audio.eva_playing = None;
        audio.eva_ended = None;
        audio.eva_delay = 500;
    }
    *state = CampaignAudioState::default();
    commands.remove_resource::<PendingCampaignAudio>();
}

fn restore_silent_audio(mut commands: Commands, pending: Option<Res<PendingCampaignAudio>>, mut state: ResMut<CampaignAudioState>) {
    if let Some(pending) = pending {
        *state = pending.0.clone();
        commands.remove_resource::<PendingCampaignAudio>();
    }
}

fn restore_campaign_audio(
    mut commands: Commands,
    pending: Option<Res<PendingCampaignAudio>>,
    mut state: ResMut<CampaignAudioState>,
    mut audio: ResMut<Audio>,
    files: Res<GameFiles>,
    time: Res<Time<Real>>,
    mut sources: ResMut<Assets<AudioSource>>,
    speaking: Query<Entity, With<Speaking>>,
) {
    let Some(pending) = pending else { return };
    for entity in &speaking { commands.entity(entity).despawn(); }
    *state = pending.0.clone();
    audio.eva_waiting = state.waiting.clone();
    audio.eva_playing = state.current_line.clone();
    audio.eva_ended = state.gap_remaining.map(|_| speech_clock(&time));
    audio.eva_delay = state.gap_remaining.map_or(500, |gap| gap.as_millis().min(500) as u64);
    if let Some(saved) = state.playing.take() {
        match audio.pcm(&files, &saved.sample).and_then(|pcm| speech_suffix(pcm, &saved)) {
            Some(pcm) => {
                let handle = sources.add(AudioSource { bytes: Arc::from(pcm.to_wav()) });
                let mut settings = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(audio.voice_volume));
                settings.paused = saved.paused;
                commands.spawn((Speaking { start_frame: saved.frame_offset, rate: saved.sample_rate }, AudioPlayer::new(handle), settings));
                audio.eva_playing = Some(saved.line.clone());
                audio.eva_ended = None;
                audio.eva_delay = 500;
                state.playing = Some(saved);
            }
            None => {
                warn!("saved campaign speech {} sample {} could not be resumed at frame {}", saved.line, saved.sample, saved.frame_offset);
                audio.eva_ended = Some(speech_clock(&time));
                audio.eva_delay = 500;
                state.gap_remaining = Some(Duration::from_millis(500));
            }
        }
    }
    commands.remove_resource::<PendingCampaignAudio>();
}

/// Slice decoded original PCM instead of rounding a frame into a nanosecond
/// seek. At e.g. 22050 Hz that round trip otherwise drops one frame.
fn speech_suffix(mut pcm: Pcm, saved: &CampaignSpeechPlayback) -> Option<Pcm> {
    if pcm.rate == 0 || pcm.channels == 0 || pcm.rate != saved.sample_rate { return None }
    let sample = usize::try_from(saved.frame_offset).ok()?.checked_mul(usize::from(pcm.channels))?;
    if sample >= pcm.samples.len() { return None }
    pcm.samples.drain(..sample);
    Some(pcm)
}

fn speech_clock(time: &Time<Real>) -> u64 {
    time.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn speech_frame(start_frame: u64, rate: u32, position: Duration) -> u64 {
    start_frame.saturating_add((position.as_nanos().saturating_mul(u128::from(rate)) / 1_000_000_000).min(u128::from(u64::MAX)) as u64)
}

fn serve_speech(audio: &mut Audio, state: &mut CampaignAudioState, files: &GameFiles, sources: &mut Assets<AudioSource>, commands: &mut Commands, now: u64, side: i32) -> Option<Entity> {
    let entry = audio.next_speech(now)?;
    // Pending lines select the current announcer at dequeue; a resumed sample
    // instead keeps the actual voice selected before the save.
    let key = announcer_key(side);
    let name = audio.eva.as_ref().and_then(|eva| eva.get(&entry.line, key)).map(|name| name.trim().to_owned());
    if let Some(name) = name {
        if let Some(handle) = audio.sample(files, sources, &name) {
            let rate = audio.decoded[&name.to_lowercase()].1;
            let entity = commands.spawn((Speaking { start_frame: 0, rate }, AudioPlayer::new(handle),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(audio.voice_volume)))).id();
            audio.eva_playing = Some(entry.line.clone());
            audio.eva_ended = None;
            audio.eva_delay = 500;
            state.playing = Some(CampaignSpeechPlayback { line: entry.line, sample: name, sample_rate: rate, frame_offset: 0, paused: false });
            return Some(entity);
        } else {
            warn!("EVA speech {} sample {name} could not be decoded", entry.line);
        }
    } else {
        warn!("EVA speech {} has no {key} sample in the mounted edition", entry.line);
    }
    None
}

/// Starts sounds and announcer lines.
// New sounds start at most
// once every 33 ms; a sound's volume is its entry's `Volume` (80 if none) times the
// caller's share, less a random `VShift`; pitch varies by `FShift`; only `random`
// in `Control` picks among the samples, otherwise the first is used; with `Limit`
// (5 if none) of an entry already playing a new one is dropped, unless it has
// `interrupt`, when the oldest is stopped instead.
// The announcer speaks one line at a time,
// the next starting strictly after the last ended plus 500 ms. The queue and
// duplicate rules follow original RA2 0x7158c0/0x7159d0/0x715ba0 and matching
// YR 0x752480/0x752590/0x752760 (native CPU traces in research/native/eva).
#[allow(clippy::too_many_arguments)]
fn play_cues(
    mut commands: Commands,
    mut cues: MessageReader<Cue>,
    mut audio: ResMut<Audio>,
    mut state: ResMut<CampaignAudioState>,
    files: Res<GameFiles>,
    time: Res<Time<Real>>,
    setup: Option<Res<crate::game::GameSetup>>,
    sim: Option<Res<crate::units::Sim>>,
    centre: Option<Res<crate::game::ViewCentre>>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut sources: ResMut<Assets<AudioSource>>,
    playing: Query<(Entity, &Playing)>,
    speaking: Query<(Entity, &Speaking, Option<&AudioSink>)>,
) {
    let now = time.elapsed_secs_f64();
    let speech_now = speech_clock(&time);
    let audio = &mut *audio;
    if audio.eva_playing.is_some() && audio.eva_ended.is_none()
        && (speaking.is_empty() || speaking.iter().all(|(_, _, sink)| sink.is_some_and(AudioSink::empty))) {
        audio.eva_ended = Some(speech_now);
        audio.eva_delay = 500;
        state.playing = None;
    }
    let side = sim.as_ref().and_then(|sim| sim.0.houses.get(crate::terrain::local() as usize)).map(|house| house.side)
        .or_else(|| setup.as_ref().and_then(|setup| audio.announcer_sides.get(setup.options.country.max(0) as usize)).copied()).unwrap_or(0);
    let mut newly_started = None;
    for cue in cues.read() {
        match cue {
            Cue::Sound(section) => audio.pending.push((section.clone(), 1.0, None)),
            Cue::SoundAt(section, share) => audio.pending.push((section.clone(), *share, None)),
            Cue::SoundOn(section, point) => audio.pending.push((section.clone(), 1.0, Some(*point))),
            Cue::Eva(_) | Cue::Speech(_) => {
                let line = match cue { Cue::Eva(line) => *line, Cue::Speech(line) => line.as_str(), _ => unreachable!() };
                if let Some(interrupted) = audio.queue_speech(line, speech_now) {
                    if interrupted {
                        for (entity, _, sink) in &speaking {
                            if let Some(sink) = sink { sink.stop(); }
                            commands.entity(entity).despawn();
                        }
                        if let Some(entity) = newly_started.take() { commands.entity(entity).despawn(); }
                        state.playing = None;
                    }
                    // Native PlayIndex polls after each request, so an idle
                    // first line starts before later requests in this frame.
                    if let Some(entity) = serve_speech(audio, &mut state, &files, &mut sources, &mut commands, speech_now, side) { newly_started = Some(entity); }
                }
            }
        }
    }
    let _ = serve_speech(audio, &mut state, &files, &mut sources, &mut commands, speech_now, side);

    state.waiting.clone_from(&audio.eva_waiting);
    state.current_line.clone_from(&audio.eva_playing);
    state.gap_remaining = audio.eva_ended.map(|end| Duration::from_millis(end.saturating_add(audio.eva_delay).saturating_sub(speech_now).min(500)));
    if audio.eva_playing.is_none() || audio.eva_ended.is_some() {
        state.playing = None;
    } else if let Some((_, speech, Some(sink))) = speaking.iter().next() {
        if let Some(saved) = &mut state.playing {
            // Rodio publishes its source cursor at approximately 5 ms
            // intervals. This is the engine's reported position, not a claim
            // to know which frame has reached the physical speaker.
            saved.frame_offset = speech_frame(speech.start_frame, speech.rate, sink.position());
            saved.paused = sink.is_paused();
        }
    }

    // Sound effects, at the service rate.
    if now - audio.serviced < 0.033 || audio.pending.is_empty() {
        return;
    }
    audio.serviced = now;
    let mut live: Vec<(Entity, String, f64)> = playing.iter().map(|(entity, sound)| (entity, sound.0.clone(), sound.1)).collect();
    // The middle of the map view and half its size, in scene pixels.
    let view = centre.map(|centre| (centre.0, Vec2::new(window.width() - 168.0, window.height() - 32.0) / 2.0));
    for (section, share, point) in std::mem::take(&mut audio.pending) {
        let Some(sounds) = &audio.sounds else {
            continue;
        };
        let get = |key: &str| sounds.get(&section, key).or_else(|| sounds.get("Defaults", key));
        let number = |key: &str, default: f32| get(key).and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok()).unwrap_or(default);
        // DATA: `Sounds` lists the samples; "$" and "#" mark where they are kept.
        let names: Vec<String> = sounds
            .get(&section, "Sounds")
            .unwrap_or("")
            .split_whitespace()
            .map(|name| name.trim_start_matches(['$', '#']).to_owned())
            .collect();
        if names.is_empty() {
            continue;
        }
        let control = get("Control").unwrap_or("").to_lowercase();
        let has = |word: &str| control.split_whitespace().any(|w| w == word);
        let (random, interrupt) = (has("random"), has("interrupt"));
        // A sound made on the map is at
        // full strength while its place is in the view (unless its `Type` has
        // LOCAL, when it fades from the middle of the view), and beyond that
        // fades to nothing over its `Range` (10 cells unless given, 60 pixels a
        // cell), counting distance up and down the screen double. A GLOBAL sound
        // is never quieter than its `MinVolume`. Under a twentieth it is not
        // played.
        // DATA: `[Defaults]` of sound.ini for keys an entry does not give.
        let share = match (point, view.as_ref()) {
            (Some(point), Some((centre, half))) => {
                let default = |key: &str| sounds.get("Defaults", key).map(|v| v.to_lowercase());
                let kind = get("Type").map(|v| v.to_lowercase()).or_else(|| default("Type")).unwrap_or_default();
                let flag = |word: &str| kind.split_whitespace().any(|w| w == word);
                let range = get("Range").or(sounds.get("Defaults", "Range")).and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok()).unwrap_or(10.0) * 60.0;
                let floor = get("MinVolume").or(sounds.get("Defaults", "MinVolume")).and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok()).unwrap_or(20.0) / 100.0;
                let (mut ax, mut ay) = ((point.x - centre.x).abs(), (point.y - centre.y).abs());
                if !flag("local") {
                    ax = (ax - half.x).max(0.0);
                    ay = (ay - half.y).max(0.0);
                }
                ay *= 2.0;
                let mut heard = if range > 0.0 && ax < range && ay < range { (range - ax.max(ay)) / range } else { 0.0 };
                if flag("global") {
                    heard = heard.max(floor.clamp(0.0, 1.0));
                }
                if heard < 0.05 {
                    continue;
                }
                share * heard
            }
            _ => share,
        };
        let limit = number("Limit", 5.0) as usize;
        let volume = (number("Volume", 80.0) / 100.0).clamp(0.0, 1.0);
        let shift = number("VShift", 0.0).clamp(0.0, 100.0) as usize;
        let pitch: Vec<i32> = get("FShift")
            .map(|v| v.split_whitespace().filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_default();

        let mut same: Vec<(Entity, f64)> = live.iter().filter(|(_, entry, _)| *entry == section).map(|&(e, _, t)| (e, t)).collect();
        if limit > 0 && same.len() >= limit {
            if !interrupt {
                continue;
            }
            same.sort_by(|a, b| a.1.total_cmp(&b.1));
            let oldest = same[0].0;
            commands.entity(oldest).despawn();
            live.retain(|(entity, ..)| *entity != oldest);
        }
        // With `attack` in `Control` the first `Attack` samples (at least one)
        // are lead-ins, and with `decay` the last `Decay` are lead-outs; one of
        // each is played before and after one of the samples between.
        let lead_in = if has("attack") { (number("Attack", 0.0) as usize).max(1).min(names.len()) } else { 0 };
        let lead_out = if has("decay") { (number("Decay", 0.0) as usize).max(1).min(names.len() - lead_in) } else { 0 };
        let body = lead_in..(names.len() - lead_out).max(lead_in + 1).min(names.len());
        let pick = body.start + if random { audio.random(body.len().max(1)) } else { 0 };
        let pick = pick.min(names.len() - 1);
        let mut sequence: Vec<usize> = Vec::new();
        if lead_in > 0 {
            sequence.push(audio.random(lead_in));
        }
        sequence.push(pick);
        if lead_out > 0 {
            sequence.push(names.len() - lead_out + audio.random(lead_out));
        }
        let quieter = 1.0 - audio.random(shift + 1) as f32 / 100.0;
        let speed = match pitch[..] {
            [low, high] if high > low => 1.0 + (low + audio.random((high - low + 1) as usize) as i32) as f32 / 100.0,
            [only, ..] => 1.0 + only as f32 / 100.0,
            [] => 1.0,
        };
        let mut handles: Vec<Handle<AudioSource>> = Vec::new();
        for &index in &sequence {
            match audio.sample(&files, &mut sources, &names[index]) {
                Some(handle) => handles.push(handle),
                None => warn!("sound {} not found", names[index]),
            }
        }
        if handles.is_empty() {
            continue;
        }
        let first = handles.remove(0);
        let level = volume * share * quieter * audio.sound_volume;
        let settings = if handles.is_empty() { PlaybackSettings::DESPAWN } else { PlaybackSettings::REMOVE };
        let mut spawned = commands.spawn((
            Playing(section.clone(), now),
            AudioPlayer::new(first),
            settings.with_volume(Volume::Linear(level)).with_speed(speed.max(0.1)),
        ));
        if !handles.is_empty() {
            spawned.insert(Chain { rest: handles, level, speed: speed.max(0.1) });
        }
        let entity = spawned.id();
        live.push((entity, section, now));
    }
}

/// Keeps music going: the menu theme in the menus, the play list in a game.
fn play_music(
    mut commands: Commands,
    mut audio: ResMut<Audio>,
    screen: Res<State<Screen>>,
    files: Res<GameFiles>,
    mut sources: ResMut<Assets<AudioSource>>,
    playing: Query<(Entity, &Music)>,
) {
    // The score screen starts the
    // theme named SCORE.
    let which = match *screen.get() {
        Screen::InGame => 1,
        Screen::Score => 2,
        _ => 0,
    };
    let in_game = which == 1;
    let mut current = false;
    for (entity, music) in &playing {
        if music.0 == which {
            current = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    if current {
        return;
    }
    let Some(themes) = &audio.themes else {
        return;
    };
    let theme = if in_game {
        let list: Vec<String> = themes
            .section("Themes")
            .iter()
            .map(|(_, name)| name.clone())
            .filter(|name| !name.is_empty())
            .filter(|name| themes.get(name, "Normal").is_none_or(|v| v.trim().eq_ignore_ascii_case("yes")))
            .collect();
        if list.is_empty() {
            return;
        }
        if audio.track == 0 {
            audio.track = 1 + audio.random(list.len());
        }
        audio.track += 1;
        list[audio.track % list.len()].clone()
    } else if which == 2 {
        "SCORE".to_owned()
    } else {
        "INTRO".to_owned()
    };
    let Some(sound) = audio.themes.as_ref().and_then(|themes| themes.get(&theme, "Sound")).map(str::to_owned)
    else {
        return;
    };
    let Some(handle) = audio.sample(&files, &mut sources, &sound) else {
        warn!("music {sound} not found");
        // Leave a silent marker so a missing track is not retried every frame.
        commands.spawn(Music(which));
        return;
    };
    let settings = if in_game { PlaybackSettings::DESPAWN } else { PlaybackSettings::LOOP };
    commands.spawn((
        Music(which),
        AudioPlayer::new(handle),
        settings.with_volume(Volume::Linear(audio.score_volume)),
    ));
}

/// Every button makes the menu click.
// DATA: rules.ini `GUIMainButtonSound`.
fn on_button(
    activate: On<bevy::ui_widgets::Activate>,
    on_press: Query<(), With<crate::shell::ClicksOnPress>>,
    audio: Option<Res<Audio>>,
    mut cues: MessageWriter<Cue>,
) {
    // The menus' big buttons have clicked already, as they went down.
    if on_press.contains(activate.entity) {
        return;
    }
    if let Some(sound) = audio.and_then(|audio| audio.button_sound.clone()) {
        cues.write(Cue::Sound(sound));
    }
}

#[cfg(test)]
mod edition_tests {
    use super::*;

    fn entry(line: &str, priority: u8, kind: SpeechKind) -> CampaignSpeechEntry {
        CampaignSpeechEntry { line: line.into(), priority, kind }
    }


    /// These expected rows were captured from unmodified Steam game.exe and
    /// gamemd.exe queue/poll instructions, running in native ELF32 probes.
    /// RA2 0x7158c0/0x7159d0/0x715ba0, YR 0x752480/0x752590/0x752760.
    /// Only the device, clock and allocator were stubbed in that oracle.
    #[test]
    fn announcer_matches_both_original_cpu_traces_10_cases_85_steps() {
        let defs = [(0,1),(0,2),(0,3),(1,0),(1,1),(1,3),(3,0),(3,3),(2,1),(0,1),(1,1),(0,1),(0,2),(0,3),(2,3)];
        let text = defs.iter().enumerate().map(|(id, &(kind, priority))| format!("[{id}]\nType={}\nPriority={}\n", ["STANDARD", "QUEUE", "INTERRUPT", "QUEUED_INTERRUPT"][kind], ["LOW", "NORMAL", "IMPORTANT", "CRITICAL"][priority])).collect::<String>();
        let mut audio = Audio {
            indices: Vec::new(), sounds: None, eva: Some(Ini::parse(text.as_bytes())),
            announcer_sides: Vec::new(), themes: None, button_sound: None,
            sound_volume: 1.0, voice_volume: 1.0, score_volume: 1.0,
            decoded: HashMap::new(), seed: 1, track: 0, pending: Vec::new(), serviced: 0.0,
            eva_waiting: Vec::new(), eva_playing: None, eva_ended: None, eva_delay: 500,
        };
        // op: 0 reset, 1 request, 2 idle channel and poll at supplied ms.
        // Expected: current ID, play count, ordinary pending, channel busy.
        let cases: &[(&str, &[(u8, u8, u16, i32, usize, bool, bool)])] = &[
            ("rank_and_drop_ordinary", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (1, 1, 0, 0, 1, true, true),
                (1, 5, 0, 0, 1, true, true),
                (1, 2, 0, 0, 1, true, true),
                (1, 6, 0, 0, 1, true, true),
                (1, 7, 0, 0, 1, true, true),
                (1, 4, 0, 0, 1, true, true),
                (1, 3, 0, 0, 1, true, true),
                (1, 9, 0, 0, 1, true, true),
                (2, 0, 1501, 6, 2, false, true),
                (2, 0, 2002, 7, 3, false, true),
                (2, 0, 2503, 2, 4, false, true),
                (2, 0, 3004, 5, 5, false, true),
                (2, 0, 3505, 4, 6, false, true),
                (2, 0, 4006, 3, 7, false, true),
                (2, 0, 4507, -1, 7, false, false),
                (2, 0, 5008, -1, 7, false, false)
            ]),
            ("ordinary_replacement", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (1, 9, 0, 0, 1, true, true),
                (1, 11, 0, 0, 1, true, true),
                (1, 1, 0, 0, 1, true, true),
                (1, 12, 0, 0, 1, true, true),
                (2, 0, 1501, 1, 2, false, true),
                (2, 0, 2002, -1, 2, false, false)
            ]),
            ("ordinary_suppressed_by_pending_high", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (1, 6, 0, 0, 1, false, true),
                (1, 1, 0, 0, 1, false, true),
                (2, 0, 1501, 6, 2, false, true),
                (2, 0, 2002, -1, 2, false, false)
            ]),
            ("critical_fifo", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (1, 2, 0, 0, 1, false, true),
                (1, 13, 0, 0, 1, false, true),
                (2, 0, 1501, 2, 2, false, true),
                (2, 0, 2002, 13, 3, false, true),
                (2, 0, 2503, -1, 3, false, false)
            ]),
            ("current_dedup_during_gap", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (2, 0, 1200, 0, 1, false, false),
                (1, 0, 0, 0, 1, false, false),
                (2, 0, 1501, -1, 1, false, false)
            ]),
            ("same_line_dedup", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (1, 0, 0, 0, 1, false, true),
                (1, 4, 0, 0, 1, false, true),
                (1, 4, 0, 0, 1, false, true),
                (1, 10, 0, 0, 1, false, true),
                (2, 0, 1501, 4, 2, false, true),
                (2, 0, 2002, 10, 3, false, true),
                (2, 0, 2503, -1, 3, false, false)
            ]),
            ("interrupt_clears", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (1, 1, 0, 0, 1, true, true),
                (1, 4, 0, 0, 1, true, true),
                (1, 6, 0, 0, 1, true, true),
                (1, 8, 0, -1, 1, true, false),
                (2, 0, 1001, 8, 2, false, true),
                (2, 0, 1502, -1, 2, false, false)
            ]),
            ("interrupt_critical_clears", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (1, 1, 0, 0, 1, true, true),
                (1, 4, 0, 0, 1, true, true),
                (1, 6, 0, 0, 1, true, true),
                (1, 14, 0, -1, 1, false, false),
                (2, 0, 1001, 14, 2, false, true),
                (2, 0, 1502, -1, 2, false, false)
            ]),
            ("same_line_interrupt_early_return", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 8, 0, 8, 1, false, true),
                (1, 1, 0, 8, 1, true, true),
                (1, 4, 0, 8, 1, true, true),
                (1, 6, 0, 8, 1, true, true),
                (1, 8, 0, 8, 1, true, true),
                (2, 0, 1501, 6, 2, false, true),
                (2, 0, 2002, 4, 3, false, true)
            ]),
            ("gap_first_has_no_privilege", &[
                (0, 0, 0, -1, 0, false, false),
                (1, 0, 0, 0, 1, false, true),
                (2, 0, 1200, 0, 1, false, false),
                (1, 4, 0, 0, 1, false, false),
                (1, 5, 0, 0, 1, false, false),
                (2, 0, 1500, 0, 1, false, false),
                (2, 0, 1501, 5, 2, false, true),
                (2, 0, 2002, 4, 3, false, true)
            ]),
        ];
        for (case, steps) in cases {
            let (mut now, mut last_start, mut played) = (1000, 1000, 0);
            for &(op, id, ms, expected, count, ordinary, busy) in *steps {
                let poll = match op {
                    0 => { audio.eva_waiting.clear(); audio.eva_playing = None; audio.eva_ended = None; audio.eva_delay = 500; false },
                    1 => audio.queue_speech(&id.to_string(), now).is_some(),
                    2 => { now = u64::from(ms); if audio.eva_playing.is_some() { audio.eva_ended = Some(last_start); } true },
                    _ => unreachable!(),
                };
                if poll && let Some(entry) = audio.next_speech(now) {
                    audio.eva_playing = Some(entry.line);
                    audio.eva_ended = None;
                    audio.eva_delay = 500;
                    last_start = now;
                    played += 1;
                }
                let current = audio.eva_playing.as_deref().and_then(|line| line.parse().ok()).unwrap_or(-1);
                assert_eq!((current, played, audio.eva_waiting.iter().any(|entry| entry.rank() == 2), audio.eva_playing.is_some() && audio.eva_ended.is_none()), (expected, count, ordinary, busy), "{case} op={op}, id={id}, ms={ms}");
            }
        }
    }

    #[test]
    fn consumed_campaign_voice_queue_and_selected_pcm_position_round_trip() {
        let state = CampaignAudioState {
            waiting: vec![entry("MissionQueued", 2, SpeechKind::Queue), entry("EVA_UnitLost", 1, SpeechKind::Standard)],
            current_line: Some("MissionPlaying".into()),
            playing: Some(CampaignSpeechPlayback { line: "MissionPlaying".into(), sample: "original_allied_voice".into(), sample_rate: 22050, frame_offset: 7, paused: true }),
            gap_remaining: Some(Duration::from_millis(375)),
        };
        let restored: CampaignAudioState = bincode::deserialize(&bincode::serialize(&state).unwrap()).unwrap();
        assert_eq!(restored, state);
        let pcm = Pcm { rate: 22050, channels: 2, samples: (0..40).collect() };
        let suffix = speech_suffix(pcm, restored.playing.as_ref().unwrap()).unwrap();
        assert_eq!(suffix.samples, (14..40).collect::<Vec<_>>());
        // Includes the restored start frame, on every later save, rather than
        // confusing the suffix sink's zero with the original sample's start.
        assert_eq!(speech_frame(7, 22050, Duration::from_millis(100)), 2212);
        assert_eq!(speech_frame(2212, 22050, Duration::from_millis(100)), 4417);
    }

    #[test]
    fn original_campaign_dialogue_resumes_at_the_saved_stereo_or_mono_frame() {
        use bevy::audio::Decodable;
        let dir = crate::files::dev_dir();
        for (edition, label) in [(crate::files::Edition::RedAlert2, "Mis_A1_EvaReachFort"), (crate::files::Edition::YuriAssets, "Mis_xA1_EvaMorePower")] {
            if crate::files::find(&dir, if edition == crate::files::Edition::RedAlert2 { "ra2.mix" } else { "ra2md.mix" }).is_none() { continue }
            let files = GameFiles::open_for_edition(&dir, edition).unwrap();
            let eva = Ini::parse(files.get("eva.ini").unwrap());
            let sample = eva.get(label, "Allied").unwrap();
            let pcm = files.audio_banks().find_map(|(index, bag)| AudioIndex::parse(index)?.decode(sample, bag))
                .or_else(|| Pcm::from_wav(files.get(&format!("{sample}.wav"))?)).unwrap();
            let frame_offset = (pcm.samples.len() / usize::from(pcm.channels) / 3) as u64;
            let saved = CampaignSpeechPlayback { line: label.into(), sample: sample.into(), sample_rate: pcm.rate, frame_offset, paused: false };
            let expected = pcm.samples[frame_offset as usize * usize::from(pcm.channels)..].to_vec();
            let suffix = speech_suffix(pcm, &saved).unwrap();
            assert_eq!(suffix.samples, expected);
            let source = AudioSource { bytes: Arc::from(suffix.to_wav()) };
            // Exercise the same Bevy decoder used by the live audio output,
            // including its very first frame after restore.
            let decoded: Vec<f32> = source.decoder().take(expected.len()).collect();
            assert_eq!(decoded.len(), expected.len());
            for (actual, original) in decoded.iter().zip(expected) {
                assert!((*actual - f32::from(original) / 32768.0).abs() <= 1.0 / 32768.0);
            }
        }
    }

    #[test]
    fn restoring_campaign_audio_replaces_old_voice_without_dequeuing_the_next_line() {
        let dir = crate::files::dev_dir();
        if crate::files::find(&dir, "ra2.mix").is_none() { return }
        let files = GameFiles::open_for_edition(&dir, crate::files::Edition::RedAlert2).unwrap();
        let eva = Ini::parse(files.get("eva.ini").unwrap());
        let sample = eva.get("Mis_A1_EvaReachFort", "Allied").unwrap().to_owned();
        let pcm = files.audio_banks().find_map(|(index, bag)| AudioIndex::parse(index)?.decode(&sample, bag))
            .or_else(|| Pcm::from_wav(files.get(&format!("{sample}.wav"))?)).unwrap();
        let saved = CampaignAudioState {
            waiting: vec![entry("EVA_UnitLost", 1, SpeechKind::Standard), entry("Mis_A1_EvaMissionObjective", 2, SpeechKind::Queue)],
            current_line: Some("Mis_A1_EvaReachFort".into()),
            playing: Some(CampaignSpeechPlayback { line: "Mis_A1_EvaReachFort".into(), sample, sample_rate: pcm.rate, frame_offset: 12345, paused: true }),
            gap_remaining: None,
        };
        let audio = Audio {
            indices: files.audio_banks().map(|(index, _)| AudioIndex::parse(index)).collect(),
            sounds: None, eva: Some(eva), announcer_sides: vec![0], themes: None, button_sound: None,
            sound_volume: 1.0, voice_volume: 1.0, score_volume: 1.0,
            decoded: HashMap::new(), seed: 1, track: 0, pending: Vec::new(), serviced: 0.0,
            eva_waiting: vec![entry("stale", 3, SpeechKind::Queue)], eva_playing: Some("stale".into()), eva_ended: None, eva_delay: 500,
        };
        let mut app = App::new();
        app.insert_resource(files).insert_resource(audio)
            .insert_resource(Time::<Real>::default())
            .init_resource::<Assets<AudioSource>>()
            .init_resource::<CampaignAudioState>()
            .insert_resource(PendingCampaignAudio(saved.clone()))
            .add_message::<Cue>()
            .add_systems(Update, (restore_campaign_audio, play_cues).chain());
        app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow));
        let stale = app.world_mut().spawn(Speaking { start_frame: 0, rate: pcm.rate }).id();
        app.update();
        assert!(app.world().get_entity(stale).is_err());
        assert!(!app.world().contains_resource::<PendingCampaignAudio>());
        assert_eq!(app.world().resource::<CampaignAudioState>(), &saved);
        assert_eq!(app.world_mut().query::<&Speaking>().iter(app.world()).count(), 1);
        app.update();
        // The source has not acquired an output sink in this headless test;
        // queued speech must remain waiting and the pending clip stay unique.
        assert_eq!(app.world().resource::<CampaignAudioState>(), &saved);
        assert_eq!(app.world_mut().query::<&Speaking>().iter(app.world()).count(), 1);
    }

    #[test]
    fn interrupt_cancels_a_voice_started_earlier_in_the_same_bevy_update() {
        let dir = crate::files::dev_dir();
        if crate::files::find(&dir, "ra2.mix").is_none() { return }
        let files = GameFiles::open_for_edition(&dir, crate::files::Edition::RedAlert2).unwrap();
        let eva = Ini::parse(files.get("eva.ini").unwrap());
        let sample = eva.get("Mis_A1_EvaReachFort", "Allied").unwrap();
        let lines = format!("[Start]\nAllied={sample}\n[Cut]\nType=INTERRUPT\nAllied={sample}\n");
        let audio = Audio {
            indices: files.audio_banks().map(|(index, _)| AudioIndex::parse(index)).collect(),
            sounds: None, eva: Some(Ini::parse(lines.as_bytes())), announcer_sides: vec![0], themes: None, button_sound: None,
            sound_volume: 1.0, voice_volume: 1.0, score_volume: 1.0,
            decoded: HashMap::new(), seed: 1, track: 0, pending: Vec::new(), serviced: 0.0,
            eva_waiting: Vec::new(), eva_playing: None, eva_ended: None, eva_delay: 500,
        };
        let mut app = App::new();
        app.insert_resource(files).insert_resource(audio).insert_resource(Time::<Real>::default())
            .init_resource::<Assets<AudioSource>>().init_resource::<CampaignAudioState>()
            .add_message::<Cue>().add_systems(Update, play_cues);
        app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow));
        app.world_mut().write_message(Cue::Speech("Start".into()));
        app.world_mut().write_message(Cue::Speech("Cut".into()));
        app.update();
        assert_eq!(app.world_mut().query::<&Speaking>().iter(app.world()).count(), 0);
        let state = app.world().resource::<CampaignAudioState>();
        assert_eq!(state.waiting, [entry("Cut", 1, SpeechKind::Interrupt)]);
        assert_eq!(state.gap_remaining, Some(Duration::ZERO));
        assert!(state.playing.is_none());
        app.world_mut().resource_mut::<Time<Real>>().advance_by(Duration::from_millis(1));
        app.update();
        assert_eq!(app.world_mut().query::<&Speaking>().iter(app.world()).count(), 1);
        assert_eq!(app.world().resource::<CampaignAudioState>().playing.as_ref().unwrap().line, "Cut");
    }

    #[test]
    fn saved_gap_retains_current_dedup_and_strict_millisecond_boundary() {
        let dir = crate::files::dev_dir();
        if crate::files::find(&dir, "ra2.mix").is_none() { return }
        let files = GameFiles::open_for_edition(&dir, crate::files::Edition::RedAlert2).unwrap();
        let eva = Ini::parse(files.get("eva.ini").unwrap());
        let sample = eva.get("Mis_A1_EvaReachFort", "Allied").unwrap();
        let lines = format!("[GapNext]\nType=QUEUE\nPriority=CRITICAL\nAllied={sample}\n");
        let audio = Audio {
            indices: files.audio_banks().map(|(index, _)| AudioIndex::parse(index)).collect(),
            sounds: None, eva: Some(Ini::parse(lines.as_bytes())), announcer_sides: vec![0], themes: None, button_sound: None,
            sound_volume: 1.0, voice_volume: 1.0, score_volume: 1.0,
            decoded: HashMap::new(), seed: 1, track: 0, pending: Vec::new(), serviced: 0.0,
            eva_waiting: Vec::new(), eva_playing: None, eva_ended: None, eva_delay: 500,
        };
        let saved = CampaignAudioState { waiting: vec![entry("GapNext", 3, SpeechKind::Queue)], current_line: Some("GapOld".into()), playing: None, gap_remaining: Some(Duration::from_millis(500)) };
        let restored: CampaignAudioState = bincode::deserialize(&bincode::serialize(&saved).unwrap()).unwrap();
        let mut app = App::new();
        app.insert_resource(files).insert_resource(audio).insert_resource(Time::<Real>::default())
            .init_resource::<Assets<AudioSource>>().init_resource::<CampaignAudioState>()
            .insert_resource(PendingCampaignAudio(restored))
            .add_message::<Cue>().add_systems(Update, (restore_campaign_audio, play_cues).chain());
        app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow));
        app.update();
        assert_eq!(app.world().resource::<CampaignAudioState>(), &saved);
        app.world_mut().resource_mut::<Time<Real>>().advance_by(Duration::from_millis(500));
        app.world_mut().write_message(Cue::Speech("GapOld".into()));
        app.update();
        let state = app.world().resource::<CampaignAudioState>();
        assert_eq!(state.current_line.as_deref(), Some("GapOld"));
        assert_eq!(state.waiting, saved.waiting);
        assert_eq!(state.gap_remaining, Some(Duration::ZERO));
        let boundary: CampaignAudioState = bincode::deserialize(&bincode::serialize(state).unwrap()).unwrap();
        app.insert_resource(PendingCampaignAudio(boundary));
        app.update();
        assert!(app.world().resource::<CampaignAudioState>().playing.is_none());
        app.world_mut().resource_mut::<Time<Real>>().advance_by(Duration::from_millis(1));
        app.update();
        let state = app.world().resource::<CampaignAudioState>();
        assert_eq!(state.playing.as_ref().unwrap().line, "GapNext");
        assert!(state.waiting.is_empty());
    }

    #[test]
    fn dynamic_mission_speech_owns_its_name_and_keeps_eva_queue_rules() {
        let mut audio = Audio {
            indices: Vec::new(), sounds: None,
            eva: Some(Ini::parse(b"[Ordinary]\nPriority=normal\n[MissionLine]\nPriority=important\nType=QUEUE\n[Replacement]\nPriority=important\n")),
            announcer_sides: Vec::new(), themes: None, button_sound: None,
            sound_volume: 1.0, voice_volume: 1.0, score_volume: 1.0,
            decoded: HashMap::new(), seed: 1, track: 0, pending: Vec::new(), serviced: 0.0,
            eva_waiting: Vec::new(), eva_playing: None, eva_ended: None, eva_delay: 500,
        };
        audio.queue_speech("Ordinary", 0);
        {
            let mission_name = String::from("MissionLine");
            audio.queue_speech(&mission_name, 0);
        }
        audio.queue_speech("MissionLine", 0);
        audio.queue_speech("missionline", 0);
        assert_eq!(audio.eva_waiting, [entry("Ordinary", 1, SpeechKind::Standard), entry("MissionLine", 2, SpeechKind::Queue)]);
        audio.queue_speech("Replacement", 0);
        assert_eq!(audio.eva_waiting[0], entry("Replacement", 2, SpeechKind::Standard));
        audio.eva_playing = Some("AlreadyPlaying".into());
        audio.queue_speech("AlreadyPlaying", 0);
        assert_eq!(audio.eva_waiting.len(), 2);
    }

    #[test]
    fn named_original_and_expansion_campaign_dialogue_decodes() {
        let dir = crate::files::dev_dir();
        for (edition, label) in [(crate::files::Edition::RedAlert2, "Mis_A1_EvaReachFort"), (crate::files::Edition::YuriAssets, "Mis_xA1_EvaMorePower")] {
            if crate::files::find(&dir, if edition == crate::files::Edition::RedAlert2 { "ra2.mix" } else { "ra2md.mix" }).is_none() { continue }
            let files = GameFiles::open_for_edition(&dir, edition).unwrap();
            let eva = Ini::parse(files.get("eva.ini").unwrap());
            let voice = eva.get(label, "Allied").expect("the installed mission dialog must name its Allied sample");
            let pcm = files.audio_banks().find_map(|(index, bag)| AudioIndex::parse(index)?.decode(voice, bag))
                .or_else(|| Pcm::from_wav(files.get(&format!("{voice}.wav"))?))
                .unwrap_or_else(|| panic!("campaign dialog {label} sample {voice} cannot be decoded"));
            assert!(!pcm.samples.is_empty());
        }
    }

    #[test]
    fn original_yr_uses_distinct_decodable_announcers_for_all_three_sides() {
        let dir = crate::files::dev_dir();
        if crate::files::find(&dir, "ra2md.mix").is_none() { return }
        let files = GameFiles::open_for_edition(&dir, crate::files::Edition::YuriAssets).unwrap();
        let rules = Ini::parse(files.get("rules.ini").unwrap());
        let eva = Ini::parse(files.get("eva.ini").unwrap());
        let indices: Vec<_> = files.audio_banks().map(|(index, _)| AudioIndex::parse(index)).collect();
        let mut voices = Vec::new();
        for (country, expected) in [(0, "Allied"), (5, "Russian"), (9, "Yuri")] {
            let key = announcer_key(crate::terrain::country_side(&rules, country));
            assert_eq!(key, expected);
            let voice = eva.get("EVA_ConstructionComplete", key).expect("each side has an original announcer");
            let pcm = indices.iter().zip(files.audio_banks()).find_map(|(index, (_, bag))| index.as_ref()?.decode(voice, bag))
                .or_else(|| Pcm::from_wav(files.get(&format!("{voice}.wav"))?))
                .unwrap_or_else(|| panic!("the selected YR {key} voice {voice} must decode"));
            assert!(!pcm.samples.is_empty());
            voices.push(voice);
        }
        assert!(voices[0] != voices[1] && voices[1] != voices[2] && voices[0] != voices[2]);
    }
}
