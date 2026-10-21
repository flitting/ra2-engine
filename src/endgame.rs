//! The end of a game: houses defeated, the victory or defeat banner and voice,
//! and the figures handed to the score screen.
//!
//! Follows the original game.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::audio::Cue;
use crate::files::GameFiles;
use crate::hud::{absolute, load, BOTTOM_BAR_HEIGHT, SIDEBAR_WIDTH};
use crate::shell::Screen;
use crate::terrain::local;
use crate::units::{Sim, UnitView};

/// A row of the score screen.
#[derive(Clone, Debug)]
pub struct ScoreRow {
    pub name: String,
    pub colour: [u8; 3],
    pub kills: u32,
    pub losses: u32,
    pub built: u32,
    pub score: i32,
}

/// The player's faction determines the native score background.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScoreSide {
    #[default]
    Allied,
    Soviet,
    Yuri,
}

impl ScoreSide {
    fn from_house_side(side: i32) -> Self {
        match side { 1 => Self::Soviet, 2 => Self::Yuri, _ => Self::Allied }
    }
}

/// What the score screen shows: the rows, best first, the game's length in
/// seconds, and how many games have been played.
#[derive(Resource, Default, Clone, Debug)]
pub struct ScoreBoard {
    pub rows: Vec<ScoreRow>,
    pub seconds: u64,
    pub games: u32,
    pub side: ScoreSide,
    /// The actual local-player result read from world.over, also used to
    /// choose retry/advance for an active campaign.
    pub won: bool,
}

/// The names of the houses, by owner, for messages and the score screen.
#[derive(Resource, Default)]
pub struct HouseNames(pub Vec<String>);

#[derive(Default)]
struct Progress {
    /// Which houses have been announced as defeated.
    told: Vec<bool>,
    /// Whether the player won, and the tick the game ended.
    result: Option<(bool, u64)>,
    /// When, in real seconds, to leave for the score screen.
    leave_at: Option<f64>,
}

fn score_due(world: &ra2_sim::World, progress: &mut Progress, now: f64) -> bool {
    if world.over.is_none() { return false; }
    // Campaign World.over is the result of the completed House.AI delay and
    // audio wait. Logic has stopped there; the original main loop dispatches
    // its scenario result without waiting for any more logical frames.
    if world.mission_data.is_some() { return true; }
    let Some((_, ended)) = progress.result else { return false; };
    if world.current_tick() < ended + 90 { return false; }
    now >= *progress.leave_at.get_or_insert(now + 4.0)
}

pub struct EndGamePlugin;

impl Plugin for EndGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScoreBoard>()
            .init_resource::<HouseNames>()
            .add_systems(Update, watch.after(crate::campaign::CampaignOutcomeSet).run_if(in_state(Screen::InGame)).run_if(resource_exists::<Sim>).run_if(resource_exists::<UnitView>));
    }
}

#[allow(clippy::too_many_arguments)]
fn watch(
    mut commands: Commands,
    sim: Res<Sim>,
    view: Res<UnitView>,
    names: Res<HouseNames>,
    files: Res<GameFiles>,
    time: Res<Time<Real>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut progress: Local<Progress>,
    mut board: ResMut<ScoreBoard>,
    mut cues: MessageWriter<Cue>,
    mut status: MessageWriter<crate::bar::Status>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let world = &sim.0;
    // A new game starts the watch afresh.
    if world.current_tick() < 5 && (progress.result.is_some() || !progress.told.is_empty()) {
        *progress = Progress::default();
    }
    progress.told.resize(world.houses.len(), false);
    let strings = files.get("ra2.csf").and_then(ra2_formats::csf::Csf::parse);
    let name_of = |house: usize| names.0.get(house).cloned().unwrap_or_default();
    // "%s has been defeated!" for 540 frames, with
    // the announcer: "player defeated", or "you have lost" for the player.
    for house in 1..world.houses.len() {
        if world.houses[house].defeated && !progress.told[house] {
            progress.told[house] = true;
            let text = strings.as_ref().and_then(|strings| strings.get("TXT_PLAYER_DEFEATED")).unwrap_or("%s has been defeated!").replacen("%s", &name_of(house), 1);
            info!("{text}");
            status.write(crate::bar::Status::Long(text, view.house_colour(house as u8)));
            cues.write(Cue::Eva(if house == local() as usize { "EVA_YouHaveLost" } else { "EVA_PlayerDefeated" }));
        }
    }
    let Some(left) = world.over.as_ref() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if progress.result.is_none() {
        let won = left.contains(&local());
        progress.result = Some((won, world.current_tick()));
        // Campaign House.Win/Lose already queued its distinct mission speech
        // when the request was accepted, before the SavourDelay/audio wait.
        if world.mission_data.is_none() {
            cues.write(Cue::Eva(if won { "EVA_YouAreVictorious" } else { "EVA_YouHaveLost" }));
            let (width, height) = (window.width() - SIDEBAR_WIDTH, window.height() - BOTTOM_BAR_HEIGHT);
            match load(&files, &mut images, "grfxtxt.shp", if won { 2 } else { 3 }, "grfxtxt.pal") {
                Some((image, size)) => {
                    commands.spawn((
                        DespawnOnExit(Screen::InGame),
                        GlobalZIndex(50),
                        absolute(((width - size.x) / 2.0).floor(), ((height - size.y) / 2.0).floor(), size),
                        ImageNode::new(image),
                        Pickable::IGNORE,
                    ));
                }
                None => warn!("grfxtxt.shp not found: no banner"),
            }
        }
    }
    if !score_due(world, &mut progress, now) { return; }
    // A row for each house; a house not defeated
    // shows its points and half as much again and a random half to whole more;
    // the highest score first.
    let mut chance = (now * 1000.0) as u64 | 1;
    let mut rows: Vec<ScoreRow> = (1..world.houses.len())
        .map(|house| {
            let figures = world.houses[house].score;
            let mut score = figures.points.max(0);
            if !world.houses[house].defeated {
                chance = chance.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let span = (score - score / 2 + 1).max(1) as u64;
                score += score / 2 + score / 2 + ((chance >> 33) % span) as i32;
            }
            ScoreRow { name: name_of(house), colour: view.house_colour(house as u8), kills: figures.kills, losses: figures.losses, built: figures.built, score }
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.score));
    board.rows = rows;
    // The clock counts sixty ticks to the second.
    board.seconds = world.current_tick() / 60;
    board.games += 1;
    board.side = world.houses.get(local() as usize).map_or(ScoreSide::Allied, |house| ScoreSide::from_house_side(house.side));
    board.won = progress.result.is_some_and(|(won, _)| won);
    *progress = Progress::default();
    next_screen.set(Screen::Score);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_side_keeps_yuri_distinct_from_allied_and_soviet() {
        assert_eq!(ScoreSide::from_house_side(0), ScoreSide::Allied);
        assert_eq!(ScoreSide::from_house_side(1), ScoreSide::Soviet);
        assert_eq!(ScoreSide::from_house_side(2), ScoreSide::Yuri);
        assert_eq!(ScoreSide::from_house_side(-1), ScoreSide::Allied);
    }

    #[test]
    fn campaign_score_waits_for_the_core_result_and_accepts_its_frozen_frame() {
        use ra2_sim::{House, Map, World, mission::*};
        for yuri in [false, true] {
            for action in [1, 2] {
                let mut world = World::new(Map::new(16, 16), None);
                world.houses = vec![House::default(), House { country: 1, human: true, ..House::default() }];
                world.attach_mission(MissionData {
                    id: "score-lifecycle".into(), yuri, player_house: 1,
                    houses: vec![HouseBinding { name: "Player House".into(), country: "Player".into(), house: 1 }],
                    country_names: vec!["Neutral".into(), "Player".into()],
                    tags: vec![TagDef { id: "result-tag".into(), trigger: "result".into(), ..TagDef::default() }],
                    triggers: vec![TriggerDef { id: "result".into(), country: "Player".into(),
                        events: vec![EventDef { kind: 8, ..EventDef::default() }],
                        actions: vec![ActionDef::with_parameter(action, "1")], ..TriggerDef::default() }],
                    ..MissionData::default()
                }).unwrap();
                world.set_mission_presentation_managed(true);
                let mut progress = Progress::default();
                for _ in 0..200 {
                    assert!(!score_due(&world, &mut progress, 10.0));
                    world.tick();
                    if world.mission_presentation_waiting() { break; }
                }
                assert!(world.mission_presentation_waiting());
                assert!(!score_due(&world, &mut progress, 10.0));
                assert!(!world.advance_mission_presentation_time(0, true));
                assert!(!score_due(&world, &mut progress, 10.0));
                assert!(world.advance_mission_presentation_time(0, false));
                let frozen_tick = world.current_tick();
                progress.result = Some((action == 1, frozen_tick));
                for _ in 0..100 { world.tick(); }
                assert_eq!(world.current_tick(), frozen_tick);
                assert!(score_due(&world, &mut progress, 10.0), "the completed campaign cannot wait for another logic frame");
                assert!(progress.leave_at.is_none());
            }
        }
    }

    #[test]
    fn skirmish_score_retains_its_existing_frame_and_wall_time_delays() {
        let mut world = ra2_sim::World::new(ra2_sim::Map::new(16, 16), None);
        world.over = Some(Vec::new());
        let mut progress = Progress { result: Some((false, 0)), ..Progress::default() };
        for _ in 0..89 { world.tick(); }
        assert!(!score_due(&world, &mut progress, 10.0));
        assert!(progress.leave_at.is_none());
        world.tick();
        assert!(!score_due(&world, &mut progress, 10.0));
        assert!(!score_due(&world, &mut progress, 13.99));
        assert!(score_due(&world, &mut progress, 14.0));
    }
}
