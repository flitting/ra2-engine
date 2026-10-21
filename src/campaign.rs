//! Campaign selection and progression, read from the installed edition's INIs.
//!
//! MAPSEL is the shipped progression. The maps' `NextScenario` entries retain
//! development-era names in RA2 and must not replace that progression.

use bevy::prelude::*;
use ra2_formats::ini::Ini;

use crate::files::{Edition, GameFiles};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CampaignSide {
    #[default]
    Allied,
    Soviet,
}

impl CampaignSide {
    pub fn index(self) -> usize {
        match self { Self::Allied => 0, Self::Soviet => 1 }
    }
}

/// The engine's DifficultyClass order, separate from the Options slider's
/// increasing-difficulty order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CampaignDifficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl CampaignDifficulty {
    pub fn engine_index(self) -> usize {
        match self { Self::Hard => 0, Self::Normal => 1, Self::Easy => 2 }
    }

    pub fn from_preference(value: i32) -> Self {
        match value { 0 => Self::Easy, 2 => Self::Hard, _ => Self::Normal }
    }
}

/// Selection identity saved alongside a campaign snapshot. `stage` is zero
/// based within the chosen side's formal missions, excluding boot camp.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CampaignRun {
    pub edition: Edition,
    pub side: CampaignSide,
    pub stage: usize,
    /// Canonical lower-case scenario filename, including its `.map` suffix.
    pub scenario: String,
    pub difficulty: CampaignDifficulty,
}

impl CampaignRun {
    pub fn map_stem(&self) -> &str {
        self.scenario.strip_suffix(".map").unwrap_or(&self.scenario)
    }

    pub fn title(&self, files: &GameFiles) -> Option<String> {
        let metadata = CampaignMetadata::load(files).ok()?;
        let stage = metadata.stage(self)?;
        let strings = ra2_formats::csf::Csf::parse(files.get("ra2.csf")?)?;
        Some(strings.get(&stage.ui_name).unwrap_or(&stage.scenario).to_owned())
    }
}

/// Present only while a selected campaign is being prepared. The map loader
/// consumes it and installs ActiveCampaign only after successful preparation.
#[derive(Resource, Clone, Debug)]
pub struct PendingCampaign(pub CampaignRun);

#[derive(Resource, Clone, Debug)]
pub struct ActiveCampaign(pub CampaignRun);

#[derive(Clone, Debug)]
pub struct CampaignBattle {
    pub id: String,
    pub side: CampaignSide,
    pub scenario: String,
    pub description: String,
    pub final_movie: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CampaignStage {
    pub id: String,
    pub scenario: String,
    pub ui_name: String,
    pub briefing: String,
    pub load_message: String,
    pub load_briefing: String,
    pub loading_picture: String,
    pub loading_palette: String,
    pub briefing_position: (i32, i32),
}

#[derive(Clone, Debug)]
pub struct CampaignMetadata {
    pub edition: Edition,
    /// Release campaigns only: excludes DebugOnly, boot camp and orphaned E3
    /// entries that remain in BATTLE.INI without DebugOnly being set.
    pub battles: Vec<CampaignBattle>,
    sides: [Vec<CampaignStage>; 2],
}

impl CampaignMetadata {
    /// Campaign metadata is optional for the rest of the game. Calling this
    /// reports a missing campaign INI without making skirmish installation
    /// checks depend on campaign maps, films or loading art.
    pub fn load(files: &GameFiles) -> Result<Self, String> {
        let read = |name: &str| files.get(name).map(Ini::parse).ok_or_else(|| format!("Campaign data is missing: {name}"));
        let battle = read("battle.ini")?;
        let mission = read("mission.ini")?;
        let mapsel = read("mapsel.ini")?;
        Self::parse(files.edition(), &battle, &mission, &mapsel)
    }

    fn parse(edition: Edition, battle: &Ini, mission: &Ini, mapsel: &Ini) -> Result<Self, String> {
        let mut sides: [Vec<CampaignStage>; 2] = Default::default();
        for (side, section) in [(CampaignSide::Allied, "GDI"), (CampaignSide::Soviet, "Nod")] {
            // Numeric keys select progression entries; Anims and Sounds are
            // presentation settings. Preserve the INI's declared order.
            for (_, id) in mapsel.section(section).iter().filter(|(key, _)| key.parse::<usize>().is_ok()) {
                let Some(scenario) = mapsel.get(id, "Scenario").filter(|name| !name.is_empty()) else { continue };
                let scenario = scenario.to_ascii_lowercase();
                if scenario.starts_with("trn") { continue }
                if !scenario.ends_with(".map") {
                    return Err(format!("Invalid campaign scenario in MAPSEL [{id}]: {scenario}"));
                }
                let value = |key: &str| mission.get(&scenario, key).unwrap_or_default().to_owned();
                let number = |key: &str, default: i32| mission.get(&scenario, key).and_then(|v| v.parse().ok()).unwrap_or(default);
                let loading_picture = value("LS800BkgdName");
                let palette = value("LS800BkgdPal");
                // RA2 1.006 0x535ddb..0x535e80 draws the mission backdrop
                // with 0x6f2470's conversion, initialized at 0x6f24ca from
                // LDSCRNA/S.PAL. MPLS.PAL belongs to multiplayer loading.
                // YR supplies a per-mission override in MISSIONMD instead.
                let loading_palette = if palette.is_empty() {
                    match side { CampaignSide::Allied => "ldscrna.pal", CampaignSide::Soviet => "ldscrns.pal" }.into()
                } else { palette };
                sides[side.index()].push(CampaignStage {
                    id: id.clone(), scenario: scenario.clone(),
                    ui_name: value("UIName"), briefing: value("Briefing"),
                    load_message: value("LSLoadMessage"), load_briefing: value("LSLoadBriefing"),
                    loading_picture, loading_palette,
                    briefing_position: (number("LS800BriefLocX", 20), number("LS800BriefLocY", 20)),
                });
            }
        }
        let mut battles = Vec::new();
        for (_, id) in battle.section("Battles") {
            if battle.get(id, "DebugOnly").is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "1")) { continue }
            let Some(scenario) = battle.get(id, "Scenario") else { continue };
            let scenario = scenario.to_ascii_lowercase();
            let Some(side) = [CampaignSide::Allied, CampaignSide::Soviet].into_iter().find(|side| sides[side.index()].first().is_some_and(|stage| stage.scenario == scenario)) else { continue };
            battles.push(CampaignBattle {
                id: id.clone(), side, scenario,
                description: battle.get(id, "Description").unwrap_or(id).to_owned(),
                final_movie: battle.get(id, "FinalMovie").filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("<none>")).map(str::to_owned),
            });
        }
        for side in [CampaignSide::Allied, CampaignSide::Soviet] {
            if sides[side.index()].is_empty() || !battles.iter().any(|battle| battle.side == side) {
                return Err(format!("No release campaign found for {side:?}"));
            }
        }
        Ok(Self { edition, battles, sides })
    }

    pub fn stages(&self, side: CampaignSide) -> &[CampaignStage] {
        &self.sides[side.index()]
    }

    pub fn run(&self, side: CampaignSide, stage: usize, difficulty: CampaignDifficulty) -> Option<CampaignRun> {
        Some(CampaignRun { edition: self.edition, side, stage, scenario: self.stages(side).get(stage)?.scenario.clone(), difficulty })
    }

    /// Validates the edition and scenario as well as the ordinal, so a saved
    /// expansion run cannot accidentally load the base game's same-numbered map.
    pub fn stage(&self, run: &CampaignRun) -> Option<&CampaignStage> {
        (self.edition == run.edition).then_some(())?;
        self.stages(run.side).get(run.stage).filter(|stage| stage.scenario.eq_ignore_ascii_case(&run.scenario))
    }

    pub fn next(&self, run: &CampaignRun) -> Option<CampaignRun> {
        self.stage(run)?;
        self.run(run.side, run.stage.checked_add(1)?, run.difficulty)
    }
}

/// Original camera action 48's interpolation. Saving this separately from the
/// current camera position preserves a pan that is in progress during a save.
#[derive(Resource, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CampaignCameraPan {
    pub start: [f32; 2],
    pub target: [f32; 2],
    pub speed_index: u8,
    pub fraction: f32,
    pub last_logic_tick: u64,
}

impl CampaignCameraPan {
    // RA2 game.exe 0x7f6914 and YR gamemd.exe 0x8428ec, read by
    // 0x6a2cd3 / 0x6d2443. Both editions use the same five f32 values.
    const SPEEDS: [f32; 5] = [0.0015, 0.003, 0.0075, 0.03, 0.06];

    fn advance(&mut self, tick: u64) -> Option<Vec2> {
        let speed = *Self::SPEEDS.get(usize::from(self.speed_index))?;
        if tick < self.last_logic_tick || !self.fraction.is_finite() || !(0.0..=1.0).contains(&self.fraction)
            || !self.start.iter().chain(&self.target).all(|v| v.is_finite()) { return None }
        for _ in self.last_logic_tick..tick {
            // Do not replace repeated f32 addition with multiplication: the
            // original stores after every logic-frame step. Action48's own
            // ftol leaves CW=0xe7f (53-bit arithmetic, stores toward zero),
            // verified through both original initializers/update functions.
            let sum = f64::from(self.fraction) + f64::from(speed);
            let nearest = sum as f32;
            self.fraction = if f64::from(nearest) > sum {
                f32::from_bits(nearest.to_bits() - 1)
            } else { nearest }.min(1.0);
            if self.fraction >= 1.0 { break }
        }
        self.last_logic_tick = tick;
        // Original 0x722140 combines integer endpoints with the f32 fraction
        // using x87 arithmetic, then 0x77f110 converts with control word
        // 0xe7f (truncate). Rounding the result to nearest shifts some pans by
        // a pixel, particularly across negative scene coordinates.
        let fraction = f64::from(self.fraction);
        let coordinate = |axis: usize| (f64::from(self.start[axis]) * (1.0 - fraction) + f64::from(self.target[axis]) * fraction).trunc() as f32;
        Some(Vec2::new(coordinate(0), coordinate(1)))
    }
}

#[derive(Resource, Default)]
struct CampaignPresentation {
    strings: Option<ra2_formats::csf::Csf>,
    warned: bool,
    outcome_wall_unit: Option<u64>,
}

pub struct CampaignPlugin;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CampaignPresentationSet;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CampaignOutcomeSet;

impl Plugin for CampaignPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CampaignPresentation>()
            .add_systems(OnEnter(crate::shell::Screen::InGame), reset_presentation)
            .add_systems(OnExit(crate::shell::Screen::InGame), clear_camera_pan)
            .add_systems(Update, (present_outputs, advance_camera_pan).chain().in_set(CampaignPresentationSet)
                .run_if(in_state(crate::shell::Screen::InGame))
                .run_if(resource_exists::<crate::units::Sim>)
                .run_if(resource_exists::<crate::units::UnitView>)
                .run_if(resource_exists::<ActiveCampaign>))
            .add_systems(Update, advance_outcome_presentation.in_set(CampaignOutcomeSet)
                .after(crate::audio::CampaignAudioSet)
                .run_if(in_state(crate::shell::Screen::InGame))
                .run_if(resource_exists::<crate::units::Sim>)
                .run_if(resource_exists::<ActiveCampaign>));
    }
}

fn reset_presentation(mut presentation: ResMut<CampaignPresentation>, files: Res<GameFiles>) {
    presentation.strings = files.get("ra2.csf").and_then(ra2_formats::csf::Csf::parse);
    presentation.warned = false;
    presentation.outcome_wall_unit = None;
}

fn advance_outcome_presentation(
    mut sim: ResMut<crate::units::Sim>,
    mut presentation: ResMut<CampaignPresentation>,
    voice: Res<crate::audio::CampaignAudioState>,
    audio: Option<Res<crate::audio::Audio>>,
    recording: Option<ResMut<crate::saves::Recording>>,
) {
    if !sim.0.mission_presentation_waiting() {
        presentation.outcome_wall_unit = None;
        return;
    }
    // Native House.AI pumps messages within its current logic frame. The
    // external audio system continues while World.tick remains frozen.
    let now = crate::game::boot_millis() >> 4;
    let elapsed = presentation.outcome_wall_unit.replace(now).map_or(0, |previous| now.saturating_sub(previous).min(u32::MAX as u64) as u32);
    let busy = audio.is_some() && (voice.playing.is_some() || !voice.waiting.is_empty());
    if let Some(mut recording) = recording {
        recording.presentation.push(crate::saves::MissionPresentationStep { tick: sim.0.current_tick(), input: crate::saves::MissionPresentationInput::OutcomeWait { elapsed_units: elapsed, audio_busy: busy } });
    }
    sim.0.advance_mission_presentation_time(elapsed, busy);
}

fn clear_camera_pan(mut commands: Commands) {
    commands.remove_resource::<CampaignCameraPan>();
}

fn present_outputs(
    mut commands: Commands,
    mut sim: ResMut<crate::units::Sim>,
    recording: Option<ResMut<crate::saves::Recording>>,
    view: Res<crate::units::UnitView>,
    centre: Res<crate::game::ViewCentre>,
    campaign: Res<ActiveCampaign>,
    mut presentation: ResMut<CampaignPresentation>,
    mut cues: MessageWriter<crate::audio::Cue>,
    mut status: MessageWriter<crate::bar::Status>,
    mut notices: MessageWriter<crate::notice::Notice>,
    mut radar_events: ResMut<crate::radar::RadarEvents>,
) {
    use ra2_sim::mission::MissionOutput;
    let tick = sim.0.current_tick();
    let outputs = sim.0.take_mission_outputs();
    if let Some(mut recording) = recording.filter(|_| !outputs.is_empty()) {
        recording.presentation.push(crate::saves::MissionPresentationStep { tick, input: crate::saves::MissionPresentationInput::DrainMissionOutputs { count: outputs.len() } });
    }
    for output in outputs {
        match output {
            MissionOutput::Text { label } => {
                let words = presentation.strings.as_ref().and_then(|strings| strings.get(&label)).unwrap_or(&label).to_owned();
                status.write(crate::bar::Status::Long(words, view.player_colour()));
            }
            MissionOutput::Sound { name } => { cues.write(crate::audio::Cue::Sound(name)); }
            MissionOutput::Speech { name } => { cues.write(crate::audio::Cue::Speech(name)); }
            MissionOutput::Camera { speed, waypoint, cell } => {
                let Ok(speed_index) = u8::try_from(speed) else { warn!(speed, waypoint, "invalid campaign camera speed"); continue };
                if CampaignCameraPan::SPEEDS.get(usize::from(speed_index)).is_none() {
                    warn!(speed, waypoint, "campaign camera speed is outside the original 0..4 table");
                    continue;
                }
                let level = i32::from(sim.0.map.level(cell.0, cell.1)) + if sim.0.map.bridged(cell.0, cell.1) { 4 } else { 0 };
                let target = crate::units::scene_point(view.origin(), cell.0 * 256 + 128, cell.1 * 256 + 128, level);
                commands.insert_resource(CampaignCameraPan { start: centre.0.to_array(), target: target.to_array(), speed_index, fraction: 0.0, last_logic_tick: tick });
            }
            MissionOutput::Diagnostic(diagnostic) => {
                warn!(scenario = campaign.0.scenario, tick = diagnostic.tick, trigger = ?diagnostic.trigger_id,
                    kind = ?diagnostic.kind, "{}", diagnostic.message);
                // Implementation diagnostics stay in the log unless explicitly
                // requested; they are not part of the original mission's HUD.
                if !presentation.warned && std::env::var("RA2_CAMPAIGN_DIAGNOSTICS").as_deref() == Ok("1") {
                    notices.write(crate::notice::Notice("Campaign fidelity gaps detected; details are in the log.".into()));
                    presentation.warned = true;
                }
            }
            MissionOutput::TriggerFired { tick, trigger_id, tag_id } => {
                debug!(tick, trigger_id, tag_id, "campaign trigger fired");
            }
            MissionOutput::Outcome(outcome) => {
                // The runtime has already set world.over. EndGamePlugin reads
                // that real result; presentation never creates a substitute.
                info!(scenario = campaign.0.scenario, ?outcome, "campaign objective result");
            }
            MissionOutput::Reveal { .. } => {}
            MissionOutput::Radar { kind, waypoint, cell } => {
                if let Some(kind) = usize::try_from(kind).ok().filter(|kind| *kind < crate::radar::EVENT_TYPES) {
                    radar_events.raise(kind, cell, tick);
                } else {
                    warn!(kind, waypoint, "campaign radar event kind is outside the original 0..16 table");
                }
            }
        }
    }
}

fn advance_camera_pan(mut commands: Commands, sim: Res<crate::units::Sim>, pan: Option<ResMut<CampaignCameraPan>>, mut centre: ResMut<crate::game::CentreOn>) {
    let Some(mut pan) = pan else { return };
    match pan.advance(sim.0.current_tick()) {
        Some(point) => centre.0 = Some(point),
        None => { warn!("invalid saved campaign camera pan"); commands.remove_resource::<CampaignCameraPan>(); return }
    }
    if pan.fraction >= 1.0 { commands.remove_resource::<CampaignCameraPan>(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_fraction_matches_both_original_full_update_cpu_traces() {
        // Captured after complete action48 + Update calls from both Steam
        // executables. The native initializer changed even CW=037f/027f to
        // 0e7f before the first update. Completion removes the live pan; here
        // its final fraction is retained as 1 until the presentation removes it.
        let original = [
            [0x3bc49ba5, 0x3cd0e55b, 0x3e199986, 0x3f3fff37, 0x3f800000],
            [0x3c449ba5, 0x3d50e55b, 0x3e999986, 0x3f800000, 0x3f800000],
            [0x3cf5c28e, 0x3e028f57, 0x3f3ffff4, 0x3f800000, 0x3f800000],
            [0x3df5c28e, 0x3f028f57, 0x3f800000, 0x3f800000, 0x3f800000],
            [0x3e75c28e, 0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000],
        ];
        for (speed, expected) in original.iter().enumerate() {
            let mut pan = CampaignCameraPan { start: [100.0, 200.0], target: [300.0, 450.0], speed_index: speed as u8, fraction: 0.0, last_logic_tick: 0 };
            for (tick, expected) in [4, 17, 100, 500, 667].into_iter().zip(expected) {
                pan.advance(tick).unwrap();
                assert_eq!(pan.fraction.to_bits(), *expected, "speed={speed} tick={tick}");
            }
        }
    }

    #[test]
    fn release_battles_exclude_debug_boot_camp_and_orphaned_e3_entries() {
        let battle = Ini::parse(b"[Battles]\n1=TUT1\n2=ALL1\n3=SOV1\n4=ALL02\n5=E31\n[TUT1]\nScenario=trn01t.map\n[ALL1]\nScenario=ALL01t.MAP\n[SOV1]\nScenario=SOV01t.MAP\n[ALL02]\nScenario=all02s.map\nDebugOnly=yes\n[E31]\nScenario=e31.map\n");
        let mapsel = Ini::parse(b"[GDI]\nAnims=Anims\n1=Trn01\n2=ALL01\n3=ALL02\n[Nod]\n1=Sov01\n[Trn01]\nScenario=trn01t.map\n[ALL01]\nScenario=ALL01t.MAP\n[ALL02]\nScenario=all02s.map\n[Sov01]\nScenario=Sov01t.MAP\n");
        let metadata = CampaignMetadata::parse(Edition::RedAlert2, &battle, &Ini::parse(b""), &mapsel).unwrap();
        assert_eq!(metadata.battles.iter().map(|battle| battle.id.as_str()).collect::<Vec<_>>(), ["ALL1", "SOV1"]);
        let run = metadata.run(CampaignSide::Allied, 0, CampaignDifficulty::Hard).unwrap();
        assert_eq!(run.map_stem(), "all01t");
        assert_eq!(metadata.next(&run).unwrap().scenario, "all02s.map");
        let mut wrong_edition = run;
        wrong_edition.edition = Edition::YuriAssets;
        assert!(metadata.stage(&wrong_edition).is_none());
    }

    #[test]
    fn campaign_save_identity_round_trips_without_changing_difficulty_order() {
        for (preference, difficulty, engine) in [(0, CampaignDifficulty::Easy, 2), (1, CampaignDifficulty::Normal, 1), (2, CampaignDifficulty::Hard, 0)] {
            assert_eq!(CampaignDifficulty::from_preference(preference), difficulty);
            assert_eq!(difficulty.engine_index(), engine);
            let run = CampaignRun { edition: Edition::YuriAssets, side: CampaignSide::Soviet, stage: 5, scenario: "sov06lmd.map".into(), difficulty };
            let bytes = bincode::serialize(&run).unwrap();
            assert_eq!(bincode::deserialize::<CampaignRun>(&bytes).unwrap(), run);
        }
    }

    #[test]
    fn original_camera_interpolation_survives_a_save_mid_pan() {
        let mut whole = CampaignCameraPan { start: [100.0, 200.0], target: [500.0, 800.0], speed_index: 3, fraction: 0.0, last_logic_tick: 20 };
        let mut saved = whole.clone();
        saved.advance(25).unwrap();
        let bytes = bincode::serialize(&saved).unwrap();
        let mut restored: CampaignCameraPan = bincode::deserialize(&bytes).unwrap();
        assert_eq!(whole.advance(30), restored.advance(30));
        assert_eq!(whole.fraction.to_bits(), restored.fraction.to_bits());
        assert_eq!(whole, restored);
        assert_eq!(whole.advance(100), Some(Vec2::new(500.0, 800.0)));
        assert_eq!(whole.fraction, 1.0);
        whole.speed_index = 5;
        assert!(whole.advance(101).is_none());
        let mut negative = CampaignCameraPan { start: [-1.0, -3.0], target: [0.0, 0.0], speed_index: 0, fraction: 0.5, last_logic_tick: 1 };
        assert_eq!(negative.advance(1), Some(Vec2::new(0.0, -1.0)), "the original conversion truncates rather than rounding to nearest");
    }

    #[test]
    fn installed_campaigns_follow_their_own_progression_and_loading_assets() {
        let dir = crate::files::dev_dir();
        if crate::files::find(&dir, "ra2.mix").is_none() { return }
        for (edition, missions, first) in [(Edition::RedAlert2, 12, "all01t.map"), (Edition::YuriAssets, 7, "all01umd.map")] {
            if edition == Edition::YuriAssets && crate::files::find(&dir, "ra2md.mix").is_none() { continue }
            let files = GameFiles::open_for_edition(&dir, edition).unwrap();
            let metadata = CampaignMetadata::load(&files).unwrap();
            assert_eq!(metadata.battles.len(), 2);
            assert_eq!(metadata.stages(CampaignSide::Allied)[0].scenario, first);
            let strings = ra2_formats::csf::Csf::parse(files.get("ra2.csf").unwrap()).unwrap();
            for side in [CampaignSide::Allied, CampaignSide::Soviet] {
                assert_eq!(metadata.stages(side).len(), missions);
                for stage in metadata.stages(side) {
                    if edition == Edition::RedAlert2 {
                        assert_eq!(stage.loading_palette, if side == CampaignSide::Allied { "ldscrna.pal" } else { "ldscrns.pal" });
                    }
                    assert!(files.get(&stage.scenario).and_then(ra2_formats::map::Map::parse).is_some(), "missing or invalid {}", stage.scenario);
                    assert!(strings.get(&stage.ui_name).is_some(), "missing title {}", stage.ui_name);
                    assert!(strings.get(&stage.briefing).is_some(), "missing briefing {}", stage.briefing);
                    assert!(files.get(&stage.loading_picture).and_then(ra2_formats::shp::Shp::parse).is_some(), "missing loading art {}", stage.loading_picture);
                    assert!(files.get(&stage.loading_palette).and_then(ra2_formats::pal::Palette::parse).is_some(), "missing loading palette {}", stage.loading_palette);
                }
                let last = metadata.run(side, missions - 1, CampaignDifficulty::Easy).unwrap();
                assert!(metadata.next(&last).is_none());
            }
        }
    }
}
