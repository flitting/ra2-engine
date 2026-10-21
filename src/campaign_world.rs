//! Building an original mission is separate from resolving a multiplayer match.
//! Country type identifiers and runtime houses deliberately have separate maps.

use std::collections::{HashMap, HashSet};

use ra2_formats::{ini::Ini, map::Map, mission::MissionDefinition};
use ra2_sim::{House, World};

use crate::{campaign::{CampaignDifficulty, CampaignRun}, files::GameFiles, terrain};

#[derive(Clone, Debug)]
pub struct InitialObject {
    pub section: &'static str,
    pub id: String,
    pub tag: Option<String>,
    pub mission: String,
    pub health: i32,
    pub veterancy: f64,
    pub group: i32,
}

#[derive(Clone, Debug)]
pub struct ScenarioHouse {
    pub name: String,
    pub country: String,
    pub original_index: String,
    pub state: House,
    pub player_control: bool,
    pub edge: i32,
}

pub struct ScenarioInit {
    pub run: CampaignRun,
    /// New interactive games choose a seed before creating objects. Headless
    /// construction uses zero for reproducibility; restore replaces the full
    /// saved generator state, so this is not an immutable mission rule.
    pub random_seed: u32,
    pub definition: MissionDefinition,
    pub houses: Vec<ScenarioHouse>,
    pub local: u8,
    /// The original country registry, including the mission's appended types.
    pub country_names: Vec<String>,
    reveal_radius: i32,
    savour_delay_ticks: i32,
    unit_mission_rates: Vec<(ra2_sim::mission::UnitMission, u32)>,
    campaign_ai_rules: ra2_sim::ai::CampaignRules,
    ai_ini: Option<Ini>,
    disabled_ai_triggers: Vec<usize>,
    global_ai_triggers: Vec<usize>,
    passive_houses: Vec<u8>,
    unit_locomotors: Vec<(u16, String)>,
    unit_open_topped: Vec<u16>,
    techno_types: Vec<ra2_sim::mission::TechnoTypeBinding>,
    building_dont_score: Vec<u16>,
    ally_reveal: bool,
    reveal_by_height: bool,
    radar_visible_to: Vec<(u8, u32)>,
    owners: HashMap<String, u8>,
}

fn yes(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "yes" | "true" | "1"))
}

fn number(value: Option<&str>, default: i32) -> i32 {
    value.and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// Preserve the native registry indices; map list keys start at zero again and
/// are not permission to replace all the stock types with the map's first type.
pub fn overlay_rules(rules: &mut Ini, map: &Ini) {
    let lists = ["Countries", "BuildingTypes", "VehicleTypes", "InfantryTypes", "AircraftTypes", "SuperWeaponTypes"];
    overlay_with_lists(rules, map, &lists);
}

fn overlay_with_lists(rules: &mut Ini, map: &Ini, lists: &[&str]) {
    let mut merged = Vec::new();
    for &name in lists {
        let mut values: Vec<String> = rules.section(name).iter().map(|(_, v)| v.clone()).collect();
        for (_, value) in map.section(name) {
            if !values.iter().any(|old| old.eq_ignore_ascii_case(value)) { values.push(value.clone()); }
        }
        merged.push((name, values));
    }
    // The raw map is kept separately too. Its country, weapon and object
    // overrides must include newly declared sections, not just stock sections.
    let mut text = String::new();
    for name in map.sections().filter(|name| !lists.iter().any(|list| name.eq_ignore_ascii_case(list))) {
        text.push_str(&format!("[{name}]\n"));
        for (key, value) in map.section(name) { text.push_str(&format!("{key}={value}\n")); }
    }
    for (name, values) in merged {
        text.push_str(&format!("[{name}]\n"));
        for (index, value) in values.iter().enumerate() { text.push_str(&format!("{index}={value}\n")); }
    }
    rules.overlay(&Ini::parse(text.as_bytes()));
}

/// Country lookups accept either the registry ID or its original Name field.
pub fn country_index(rules: &Ini, name: &str) -> Option<usize> {
    rules.section("Countries").iter().position(|(_, id)| {
        id.eq_ignore_ascii_case(name.trim()) || rules.get(id, "Name").is_some_and(|label| label.eq_ignore_ascii_case(name.trim()))
    })
}

/// ParentCountry is one textual lookup, not an inheritance graph. The native
/// owner check uses `1 << lookup_result`: an unknown parent returns -1, whose
/// x86 shift count becomes bit 31. A neutral country's self-parent is valid.
pub fn build_country(rules: &Ini, name: &str) -> Result<usize, String> {
    let index = country_index(rules, name).ok_or_else(|| format!("unknown country {name}"))?;
    let id = &rules.section("Countries")[index].1;
    let parent = rules.get(id, "ParentCountry").unwrap_or(id);
    Ok(if parent.eq_ignore_ascii_case("<random>") { 30 } else { country_index(rules, parent).map_or(31, |index| index & 31) })
}

impl ScenarioInit {
    pub fn parse(files: &GameFiles, map: &Map, run: &CampaignRun) -> Result<Self, String> {
        if run.edition != files.edition() { return Err("campaign edition differs from mounted game files".into()); }
        let mut rules = files.get("rules.ini").map(Ini::parse).ok_or("campaign rules missing")?;
        overlay_rules(&mut rules, &map.ini);
        // The originals register global AI types before the scenario adds its
        // types. Keep ID bindings intact instead of pointing global triggers
        // into an unrelated map-only TeamTypes array.
        let mut ai = files.get("ai.ini").map(Ini::parse).unwrap_or_else(|| Ini::parse(b""));
        let global_ids: HashSet<_> = ai.section("AITriggerTypes").iter().map(|(id, _)| id.to_ascii_uppercase()).collect();
        let map_ai_ids: HashSet<_> = map.ini.section("AITriggerTypes").iter().map(|(id, _)| id.to_ascii_uppercase()).collect();
        overlay_with_lists(&mut ai, &map.ini, &["TaskForces", "ScriptTypes", "TeamTypes"]);
        let mut init = Self::from_definition(&rules, MissionDefinition::parse(&ai).map_err(|e| e.to_string())?, run)?;
        for (index, (id, _)) in ai.section("AITriggerTypes").iter().filter(|(_, line)| line.split(',').count() >= 7).enumerate() {
            let is_original_global = global_ids.contains(&id.to_ascii_uppercase());
            // New map trigger types start disabled. A map override of an
            // existing global retains its enabled value until Enable is read,
            // but ReadINI always rewrites its IsGlobal flag to false.
            if !ai.get("AITriggerTypesEnable", id).map_or(is_original_global, |value| yes(Some(value))) { init.disabled_ai_triggers.push(index); }
            if is_original_global && !map_ai_ids.contains(&id.to_ascii_uppercase()) { init.global_ai_triggers.push(index); }
        }
        init.ai_ini = Some(ai);
        Ok(init)
    }

    fn from_definition(rules: &Ini, definition: MissionDefinition, run: &CampaignRun) -> Result<Self, String> {
        if definition.houses.is_empty() { return Err("mission has no Houses list".into()); }
        let mut houses = vec![ScenarioHouse { name: "Neutral".into(), country: "Neutral".into(), original_index: String::new(),
            state: House::default(), player_control: false, edge: -1 }];
        let mut owners = HashMap::new();
        for (original_house_index, source) in definition.houses.iter().enumerate() {
            let country = source.get("Country").unwrap_or(&source.name).trim();
            let native_country = country_index(rules, country).ok_or_else(|| format!("unknown country {country}"))?;
            if native_country > 31 { return Err(format!("country {country} exceeds current ownership mask capacity")); }
            let side = match rules.get(country, "Side").unwrap_or("GDI").to_ascii_lowercase().as_str() {
                "nod" => 1, "thirdside" => 2, _ => 0,
            };
            let player_control = yes(source.get("PlayerControl"));
            // Both original House.ReadINI branches use a signed upper-bound
            // check: values above MaxIQLevels become 1; negatives survive.
            let authored_iq = number(source.get("IQ"), 0);
            let iq = if authored_iq > number(rules.get("IQ", "MaxIQLevels"), 5) { 1 } else { authored_iq };
            let mut brain = ra2_sim::ai::Brain::for_campaign(run.difficulty.engine_index(), ra2_sim::ai::CampaignState {
                iq, production: yes(source.get("Production")),
                autocreate: yes(source.get("Autocreate")), auto_base_building: yes(source.get("AutoBaseBuilding")),
            });
            brain.triggers_active = yes(source.get("AITriggersActive"));
            brain.original_house_index = Some(original_house_index as u32);
            brain.trigger_ratio = number(source.get("RatioAITriggerTeam"), 100);
            for index in 0..number(source.get("NodeCount"), 0).max(0) {
                let Some(row) = source.get(&format!("{index:03}")) else { continue };
                let fields: Vec<_> = row.split(',').map(str::trim).collect();
                if let [kind, x, y, ..] = fields.as_slice() {
                    if let (Some(kind), Ok(x), Ok(y)) = (rules.section("BuildingTypes").iter().position(|(_, id)| id.eq_ignore_ascii_case(kind)), x.parse(), y.parse()) {
                        brain.nodes.push(ra2_sim::ai::Node::planned(kind as u16, (x, y)));
                    }
                }
            }
            let house = ScenarioHouse {
                name: source.name.clone(), country: country.to_owned(), original_index: source.id.clone(), player_control,
                edge: match source.get("Edge").unwrap_or("").to_ascii_lowercase().as_str() { "" if run.edition == crate::files::Edition::RedAlert2 => 0, "north" => 0, "east" => 1, "south" => 2, "west" => 3, "air" => 4, _ => -1 },
                state: House { country: native_country as u8, parent_country: Some(build_country(rules, country)? as u8), side,
                    credits: number(source.get("Credits"), 0).saturating_mul(100),
                    tech_level: number(source.get("TechLevel"), 10), human: player_control,
                    // The campaign branch uses authored switches and Base
                    // nodes; it does not invent a skirmish base or sell scenery.
                    ai: (!player_control && !country.eq_ignore_ascii_case("Neutral")).then_some(brain), ..House::default() },
            };
            let at = if country.eq_ignore_ascii_case("Neutral") { houses[0] = house; 0 } else {
                if houses.len() >= 32 { return Err("mission exceeds current 32-house alliance capacity".into()); }
                houses.push(house); houses.len() - 1
            };
            if owners.insert(source.name.to_ascii_uppercase(), at as u8).is_some() { return Err(format!("duplicate mission house {}", source.name)); }
        }
        owners.entry("NEUTRAL".into()).or_insert(0);
        // A Country reference selects the first matching runtime house. Do not
        // overwrite a real House name that happens to equal another Country.
        for (at, house) in houses.iter().enumerate() {
            owners.entry(house.country.to_ascii_uppercase()).or_insert(at as u8);
        }
        let local_name = definition.basic.player.as_deref().ok_or("mission Basic.Player is absent")?;
        let local = *owners.get(&local_name.to_ascii_uppercase()).ok_or_else(|| format!("unknown mission player {local_name}"))?;
        houses[local as usize].state.human = true;
        houses[local as usize].state.ai = None;
        for source in &definition.houses {
            let at = owners[&source.name.to_ascii_uppercase()] as usize;
            // Native ReadINI applies this list through MakeAlly, whose entry
            // gate rejects self even when explicitly named. IsAllied supplies
            // self-alliance independently of the stored mask.
            let mut allies = 0u32;
            if let Some(list) = source.get("Allies") {
                for name in list.split(',').map(str::trim).filter(|v| !v.is_empty()) {
                    let id = owners.get(&name.to_ascii_uppercase()).ok_or_else(|| format!("{} has unknown ally {name}", source.name))?;
                    if *id as usize != at { allies |= 1u32 << *id; }
                }
            }
            houses[at].state.allies = allies;
        }
        let country_names = rules.section("Countries").iter().map(|(_, v)| v.clone()).collect();
        let unit_mission_rates = rules.get("Hunt", "Rate").and_then(|value| value.parse::<f64>().ok()).filter(|minutes| minutes.is_finite() && *minutes >= 0.0)
            .map(|minutes| vec![(ra2_sim::mission::UnitMission::Hunt, (minutes * 900.0).trunc() as u32)]).unwrap_or_default();
        let reveal_radius = number(rules.get("General", "RevealTriggerRadius"), 9);
        let savour_delay_ticks = ra2_sim::mission::savour_delay_ticks(rules.get("General", "SavourDelay").and_then(|value| value.parse().ok()).unwrap_or(0.1));
        let passive_houses = houses.iter().enumerate().filter(|(_, house)| yes(rules.get(&house.country, "MultiplayPassive"))).map(|(index, _)| index as u8).collect();
        let ally_reveal = rules.get("General", "AllyReveal").map_or(true, |value| yes(Some(value)));
        let reveal_by_height = rules.get("General", "RevealByHeight").map_or(true, |value| yes(Some(value)));
        let radar_visible_to = definition.houses.iter().filter_map(|source| {
            let mask = source.get("RadarVisibleTo")?.split(',').map(str::trim).filter_map(|name| {
                if name.eq_ignore_ascii_case("<random>") { Some(30) } else { country_index(rules, name).map(|index| index & 31) }
            }).fold(0u32, |mask, index| mask | (1 << index));
            Some((owners[&source.name.to_ascii_uppercase()], mask))
        }).collect();
        let unit_locomotors = ["VehicleTypes", "InfantryTypes", "AircraftTypes"].iter().flat_map(|section| rules.section(section)).enumerate()
            .filter_map(|(index, (_, id))| rules.get(id, "Locomotor").map(|guid| (index as u16, guid.to_owned()))).collect();
        let unit_open_topped = ["VehicleTypes", "InfantryTypes", "AircraftTypes"].iter().flat_map(|section| rules.section(section)).enumerate()
            .filter(|(_, (_, id))| run.edition == crate::files::Edition::YuriAssets && yes(rules.get(id, "OpenTopped"))).map(|(index, _)| index as u16).collect();
        let techno_types = rules.section("BuildingTypes").iter().enumerate()
            .map(|(index, (_, id))| ra2_sim::mission::TechnoTypeBinding { id: id.trim().to_owned(), kind: ra2_sim::mission::MissionTechnoKind::Building(index as u16) })
            .chain(["VehicleTypes", "InfantryTypes", "AircraftTypes"].iter().flat_map(|section| rules.section(section)).enumerate()
                .map(|(index, (_, id))| ra2_sim::mission::TechnoTypeBinding { id: id.trim().to_owned(), kind: ra2_sim::mission::MissionTechnoKind::Unit(index as u16) })).collect();
        let building_dont_score = rules.section("BuildingTypes").iter().enumerate()
            .filter(|(_, (_, id))| yes(rules.get(id, "DontScore"))).map(|(index, _)| index as u16).collect();
        let defaults = ra2_sim::ai::CampaignRules::default();
        let campaign_ai_rules = ra2_sim::ai::CampaignRules {
            production: number(rules.get("IQ", "Production"), defaults.production),
            guard_area: number(rules.get("IQ", "GuardArea"), defaults.guard_area),
            repair_sell: number(rules.get("IQ", "RepairSell"), defaults.repair_sell),
            harvester: number(rules.get("IQ", "Harvester"), defaults.harvester),
            aircraft: number(rules.get("IQ", "Aircraft"), defaults.aircraft),
            super_weapons: number(rules.get("IQ", "SuperWeapons"), defaults.super_weapons),
            sell_back: number(rules.get("IQ", "SellBack"), defaults.sell_back),
        };
        Ok(Self { run: run.clone(), random_seed: 0, definition, houses, local, country_names, owners, reveal_radius, savour_delay_ticks, unit_mission_rates, campaign_ai_rules, ai_ini: None, disabled_ai_triggers: Vec::new(), global_ai_triggers: Vec::new(), passive_houses, unit_locomotors, unit_open_topped, techno_types, building_dont_score, ally_reveal, reveal_by_height, radar_visible_to })
    }

    pub fn owner(&self, name: &str) -> Result<u8, String> {
        self.owners.get(&name.to_ascii_uppercase()).copied().ok_or_else(|| format!("unknown initial owner {name}"))
    }

    pub fn player(&self) -> terrain::Player {
        terrain::Player { country: self.houses[self.local as usize].state.country as usize,
            colour: 0, opponents: Vec::new(), mode_rules: String::new(), start: None }
    }

    pub fn local_position(&self) -> Option<(i32, i32)> {
        let home = number(self.definition.basic.get("HomeCell"), 98);
        self.definition.waypoints.iter().find(|w| w.id == home).map(|w| (w.x, w.y))
    }
}

pub fn build_scene(files: &GameFiles, map: &Map, run: &CampaignRun) -> Result<terrain::Scene, String> {
    let init = ScenarioInit::parse(files, map, run)?;
    // Validate owners before entering the atlas builder (which cannot report a
    // typed loading error). Missing art is diagnosed independently.
    for owner in map.units.iter().chain(&map.aircraft).map(|o| o.owner.as_str()).chain(map.infantry.iter().map(|o| o.owner.as_str())).chain(map.structures.iter().map(|o| o.owner.as_str())) { init.owner(owner)?; }
    let player = init.player();
    terrain::build_campaign(files, map, &player, init).ok_or_else(|| format!("could not build campaign {}", run.scenario))
}

/// Mission TeamTypes use the edition's ai.ini merged with the map. The lossless
/// MissionDefinition remains available separately: execution follows the
/// native numeric key ranges (six task-force rows, fifty script rows), skips
/// missing rows, and drops task-force rows whose unit type cannot be resolved.
/// See native TaskForce/ScriptType ReadINI evidence for both original editions.
pub fn team_data(init: &ScenarioInit, map: &Ini, rules: &Ini, unit: &dyn Fn(&str) -> Option<u16>, building: &dyn Fn(&str) -> Option<u16>, country: &dyn Fn(&str) -> Option<u8>) -> ra2_sim::teams::Data {
    crate::aidata::load(init.ai_ini.as_ref().unwrap_or(map), rules, unit, building, country)
}

fn runtime_event(event: &ra2_formats::mission::EventDefinition) -> ra2_sim::mission::EventDef {
    let parameter = event.value.parse().unwrap_or(0);
    // Event23's third field names a TeamType, including all-numeric IDs with
    // leading zeros. Events60/61 instead carry a separate TechnoType field.
    let argument = event.techno_type.clone().unwrap_or_else(|| {
        if event.opcode == 23 || event.value.parse::<i32>().is_err() { event.value.clone() } else { String::new() }
    });
    ra2_sim::mission::EventDef { kind: event.opcode, parameter, argument }
}

pub fn attach(world: &mut World, init: &ScenarioInit, objects: &[(InitialObject, ra2_sim::combat::Aim)]) -> Result<(), String> {
    use ra2_sim::mission::*;
    let definition = &init.definition;
    // World reserves runtime house zero for Neutral. Original House.Array
    // keeps the authored order, which also controls periodic House tag calls
    // and first-country lookup; retain it separately from runtime IDs.
    let houses = definition.houses.iter().map(|source| {
        init.owner(&source.name).map(|index| {
            let house = &init.houses[index as usize];
            HouseBinding { name: house.name.clone(), country: house.country.clone(), house: index }
        })
    }).collect::<Result<Vec<_>, _>>()?;
    let mut data = MissionData {
        id: format!("{:?}/{}", init.run.edition, init.run.scenario),
        player_house: init.local,
        human_player_houses: Some(vec![init.local]),
        yuri: init.run.edition == crate::files::Edition::YuriAssets,
        difficulty: match init.run.difficulty { CampaignDifficulty::Easy => 0, CampaignDifficulty::Normal => 1, CampaignDifficulty::Hard => 2 },
        country_names: init.country_names.clone(),
        unit_mission_rates: init.unit_mission_rates.clone(),
        reveal_radius: init.reveal_radius,
        savour_delay_ticks: init.savour_delay_ticks,
        campaign_ai_rules: init.campaign_ai_rules.clone(),
        disabled_ai_triggers: init.disabled_ai_triggers.clone(),
        global_ai_triggers: init.global_ai_triggers.clone(),
        passive_houses: init.passive_houses.clone(),
        house_edges: init.houses.iter().enumerate().map(|(index, house)| (index as u8, house.edge)).collect(),
        unit_locomotors: init.unit_locomotors.clone(),
        unit_open_topped: init.unit_open_topped.clone(),
        techno_types: init.techno_types.clone(),
        building_dont_score: init.building_dont_score.clone(),
        ally_reveal: init.ally_reveal,
        reveal_by_height: init.reveal_by_height,
        radar_visible_to: init.radar_visible_to.clone(),
        ignore_global_ai_triggers: yes(definition.basic.get("IgnoreGlobalAITriggers")),
        local_variable_limit: if init.run.edition == crate::files::Edition::YuriAssets { 100 } else { 50 },
        objects: objects.iter().map(|(initial, object)| ObjectBinding { section: initial.section.to_owned(), id: initial.id.clone(), tag: initial.tag.clone(), object: *object, group: initial.group, initial_mission: initial.mission.clone() }).collect(),
        cell_tags: definition.cell_tags.iter().map(|tag| ((tag.x, tag.y), tag.tag_id.clone())).collect(),
        houses,
        tags: definition.tags.iter().map(|tag| TagDef { id: tag.id.clone(), name: tag.name.clone(), trigger: tag.trigger_id.clone(), persistence: tag.repeat.raw_value() as u8 }).collect(),
        local_variables: definition.variables.iter().map(|v| (v.id, v.initial_value != 0)).collect(),
        waypoints: definition.waypoints.iter().map(|w| (w.id, (w.x, w.y))).collect(),
        team_types: definition.team_types.iter().enumerate().map(|(kind, team)| (team.id.clone(), kind)).collect(),
        team_definitions: definition.team_types.iter().enumerate().map(|(kind, team)| {
            let force_group = team.task_force.as_ref().and_then(|id| definition.task_forces.iter().find(|force| force.id.eq_ignore_ascii_case(id))).and_then(|force| force.group).unwrap_or(-1);
            let own_group = number(team.get("Group"), -1);
            TeamDef { kind, house_country: team.house.clone().unwrap_or_default(),
                waypoint: team.waypoint.as_deref().and_then(ra2_formats::mission::waypoint_index),
                transport_waypoint: team.transport_waypoint.as_deref().and_then(ra2_formats::mission::waypoint_index),
                tag: team.tag.clone(), group: if own_group == -1 { force_group } else { own_group },
                loose_recruit: yes(team.get("LooseRecruit")), properties: team.properties.clone() }
        }).collect(),
        ..MissionData::default()
    };
    for trigger in &definition.triggers {
        let events = definition.events.iter().find(|e| e.trigger_id.eq_ignore_ascii_case(&trigger.id));
        let actions = definition.actions.iter().find(|a| a.trigger_id.eq_ignore_ascii_case(&trigger.id));
        data.triggers.push(TriggerDef { id: trigger.id.clone(), country: trigger.country.clone(), linked: trigger.linked_trigger.clone(), enabled: !trigger.disabled, must_transfer: trigger.must_transfer,
            difficulty: trigger.difficulty,
            events: events.into_iter().flat_map(|e| &e.events).map(runtime_event).collect(),
            actions: actions.into_iter().flat_map(|a| &a.actions).map(|a| ActionDef { kind: a.opcode, parameters: [a.parameter_type.to_string(), a.parameter.clone(), a.arguments[0].clone(), a.arguments[1].clone(), a.arguments[2].clone(), a.arguments[3].clone(), a.waypoint.clone()] }).collect() });
    }
    world.attach_mission(data)
}

pub fn world(files: &GameFiles, run: &CampaignRun) -> Result<World, String> {
    let parsed = files.get(&run.scenario).and_then(Map::parse).ok_or_else(|| format!("could not parse {}", run.scenario))?;
    let mut scene = build_scene(files, &parsed, run)?;
    let mut world = crate::units::build_world(files, &mut scene).world;
    attach(&mut world, scene.scenario.as_ref().ok_or("scenario initialization missing")?, &scene.scenario_objects)?;
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{campaign::{CampaignSide, CampaignDifficulty}, files::Edition};

    fn run() -> CampaignRun { CampaignRun { edition: Edition::RedAlert2, side: CampaignSide::Soviet, stage: 0, scenario: "sov01t.map".into(), difficulty: CampaignDifficulty::Normal } }

    #[test]
    fn original_yr_reactor_damage_fires_bind_assets_and_survive_restore() {
        let dir = crate::files::dev_dir();
        if !dir.exists() { return; }
        let files = GameFiles::open_for_edition(&dir, Edition::YuriAssets).unwrap();
        let run = CampaignRun { edition: Edition::YuriAssets, side: CampaignSide::Allied, stage: 0,
            scenario: "all01umd.map".to_owned(), difficulty: CampaignDifficulty::Normal };
        let mut game = world(&files, &run).unwrap();
        assert!(game.damage_fire_rules.enabled);
        assert_eq!(game.damage_fire_rules.types.iter().map(|kind| (kind.id.as_str(), kind.end, kind.rate)).collect::<Vec<_>>(),
            [("FIRE01", 30, 2), ("FIRE02", 64, 2), ("FIRE03", 30, 2)]);
        for _ in 0..15 { game.tick(); }
        let reactor = game.buildings().iter().find(|building| building.cell == (55, 72)).unwrap();
        assert_eq!(reactor.health, 269);
        assert!(reactor.damage_fires_active);
        let fires: Vec<_> = reactor.damage_fires.iter().flatten().collect();
        assert_eq!(fires.len(), 2);
        assert_eq!((fires[0].coords, fires[0].z_adjust), ((14515, 18253, 416), -168));
        assert_eq!((fires[1].coords, fires[1].z_adjust), ((14498, 19225, 416), -84));
        assert_eq!(fires[1].type_index as usize, (fires[0].type_index as usize + 1) % 3);
        let save = game.snapshot();
        let mut restored = world(&files, &run).unwrap();
        restored.restore(&save).unwrap();
        for _ in 0..130 { game.tick(); restored.tick(); }
        assert_eq!(game.state_hash(), restored.state_hash(), "fire timers, RNG and loop phase must survive a new World");
    }

    #[test]
    fn leaving_map_event_keeps_numeric_team_identity() {
        let map = Ini::parse(b"[Events]\nT=2,23,0,001234,13,0,15\n");
        let definition = MissionDefinition::parse(&map).unwrap();
        let source = &definition.events[0].events;
        assert_eq!(runtime_event(&source[0]).argument, "001234");
        assert_eq!(runtime_event(&source[1]).parameter, 15);
        assert!(runtime_event(&source[1]).argument.is_empty());
    }

    #[test]
    fn custom_country_identity_is_distinct_from_parent_build_permissions() {
        let mut rules = Ini::parse(b"[Countries]\n0=Americans\n1=Russians\n2=Neutral\n[Americans]\nSide=GDI\n[Russians]\nSide=Nod\n");
        let map = Ini::parse(b"[Countries]\n0=Player\n1=Friendly\n[Player]\nParentCountry=Americans\nSide=Nod\n[Friendly]\nParentCountry=Russians\n[Basic]\nPlayer=Player House\n[Houses]\n0=Player House\n1=Friendly House\n[Player House]\nCountry=Player\nCredits=25\nAllies=Player House,Friendly House\n[Friendly House]\nCountry=Friendly\nCredits=10\n");
        overlay_rules(&mut rules, &map);
        let init = ScenarioInit::from_definition(&rules, MissionDefinition::parse(&map).unwrap(), &run()).unwrap();
        assert_eq!(init.country_names, ["Americans", "Russians", "Neutral", "Player", "Friendly"]);
        assert_eq!(init.owner("Player").unwrap(), init.owner("Player House").unwrap());
        let mine = &init.houses[init.local as usize];
        assert_eq!((mine.state.country, mine.state.parent_country, mine.state.side, mine.state.credits), (3, Some(0), 1, 2500));
        assert_eq!(mine.state.country as usize, init.country_names.iter().position(|v| v == "Player").unwrap());
        assert!(mine.state.allies & (1 << init.owner("Friendly House").unwrap()) != 0);
    }

    #[test]
    fn campaign_alliance_mask_does_not_invent_self_or_reverse_bits() {
        let rules = Ini::parse(b"[Countries]\n0=Americans\n1=Russians\n2=Neutral\n");
        let map = Ini::parse(b"[Basic]\nPlayer=Player\n[Houses]\n0=Player\n1=Friendly\n2=Neutral\n[Player]\nCountry=Americans\nAllies=Player,Friendly\n[Friendly]\nCountry=Russians\n[Neutral]\nCountry=Neutral\nAllies=\n");
        for edition in [Edition::RedAlert2, Edition::YuriAssets] {
            let mut run = run();
            run.edition = edition;
            let init = ScenarioInit::from_definition(&rules, MissionDefinition::parse(&map).unwrap(), &run).unwrap();
            let player = init.owner("Player").unwrap();
            let friendly = init.owner("Friendly").unwrap();
            let neutral = init.owner("Neutral").unwrap();
            let mut world = World::new(ra2_sim::Map::new(16, 16), None);
            world.houses = init.houses.into_iter().map(|house| house.state).collect();
            assert_eq!(world.houses[player as usize].allies, 1u32 << friendly);
            assert_eq!(world.houses[friendly as usize].allies, 0);
            assert_eq!(world.houses[neutral as usize].allies, 0);
            // A zero raw self bit must not make a house hostile to itself.
            for house in [player, friendly, neutral] { assert!(world.allied(house, house)); }
            assert!(world.allied(player, friendly));
            assert!(!world.allied(friendly, player));
            assert!(!world.allied(player, neutral));
        }
    }

    #[test]
    fn parent_country_is_one_lookup_and_keeps_original_unknown_shift_behavior() {
        let rules = Ini::parse(b"[Countries]\n0=Americans\n1=Neutral\n2=Civilian1\n3=Child\n[Americans]\nName=United States\n[Neutral]\nParentCountry=Neutral\n[Civilian1]\nParentCountry=Civie1 House\n[Child]\nParentCountry=Civilian1\n");
        assert_eq!(build_country(&rules, "Americans").unwrap(), 0);
        assert_eq!(build_country(&rules, "Neutral").unwrap(), 1);
        assert_eq!(build_country(&rules, "Civilian1").unwrap(), 31);
        assert_eq!(build_country(&rules, "Child").unwrap(), 2);
        assert_eq!(country_index(&rules, "United States"), Some(0));
    }

    #[test]
    fn campaign_iq_keeps_native_signed_upper_bound_behavior() {
        // Original RA2 4ebe10..4ebe2f / YR 500da7..500dc6 compare with JLE,
        // write 1 only above the configured maximum, and preserve negatives.
        for (maximum, authored, expected) in [(5, -1, -1), (5, 0, 0), (5, 5, 5), (5, 6, 1), (5, i32::MAX, 1), (-2, -3, -3), (-2, -2, -2), (-2, -1, 1)] {
            let rules = Ini::parse(format!("[Countries]\n0=Americans\n1=Russians\n[IQ]\nMaxIQLevels={maximum}\n").as_bytes());
            let map = Ini::parse(format!("[Basic]\nPlayer=Player\n[Houses]\n0=Player\n1=Enemy\n[Player]\nCountry=Americans\n[Enemy]\nCountry=Russians\nIQ={authored}\n").as_bytes());
            let init = ScenarioInit::from_definition(&rules, MissionDefinition::parse(&map).unwrap(), &run()).unwrap();
            let enemy = &init.houses[init.owner("Enemy").unwrap() as usize];
            assert_eq!(enemy.state.ai.as_ref().unwrap().campaign.as_ref().unwrap().iq, expected);
        }
    }

    #[test]
    fn initial_camera_uses_basic_home_waypoint_instead_of_waypoint_zero() {
        let rules = Ini::parse(b"[Countries]\n0=Americans\n[Americans]\nSide=GDI\n");
        let map = Ini::parse(b"[Basic]\nPlayer=Player House\nHomeCell=14\n[Houses]\n0=Player House\n[Player House]\nCountry=Americans\n[Waypoints]\n0=1001\n14=110100\n");
        let init = ScenarioInit::from_definition(&rules, MissionDefinition::parse(&map).unwrap(), &run()).unwrap();
        assert_eq!(init.local_position(), Some((100, 110)));
    }

    #[test]
    fn campaign_team_execution_uses_native_bounds_while_source_keeps_extra_rows() {
        let mut rules = Ini::parse(b"[Countries]\n0=Americans\n[Americans]\nSide=GDI\n");
        let map = Ini::parse(b"[Basic]\nPlayer=Player House\n[Houses]\n0=Player House\n[Player House]\nCountry=Americans\n[TaskForces]\n0=Force\n[Force]\nGroup=6\n0=1,GI\n6=2,MISSING\n[ScriptTypes]\n0=Script\n[Script]\n0=4,12\n50=49,not-a-number\n[TeamTypes]\n0=Team\n[Team]\nTaskForce=Force\nScript=Script\nHouse=Americans\nWaypoint=AA\nGroup=-1\n");
        overlay_rules(&mut rules, &map);
        let init = ScenarioInit::from_definition(&rules, MissionDefinition::parse(&map).unwrap(), &run()).unwrap();
        let data = team_data(&init, &map, &rules, &|name| (name == "GI").then_some(5), &|_| None, &|_| Some(0));
        assert_eq!(data.task_forces, [vec![(1, 5)]]);
        assert_eq!(data.scripts, [vec![(4, 12)]]);
        assert_eq!(init.definition.task_forces[0].members.len(), 2);
        assert_eq!(init.definition.scripts[0].lines.len(), 2);
        assert_eq!(data.team_types[0].script, 0);
        assert_eq!(data.team_types[0].task_force, 0);
    }

    /// This is an integration test against the owner's installation, not a
    /// substitute for the separate original-executable behavior comparison.
    #[test]
    #[ignore = "requires the Steam RA2 and Yuri's Revenge assets; builds four original scenes"]
    fn original_campaign_houses_objects_and_resume_are_integrated() {
        use crate::campaign::{CampaignMetadata, CampaignSide};
        use ra2_sim::{combat::Aim, mission::MissionOutput};
        let dir = crate::files::dev_dir();
        assert!(crate::files::find(&dir, "ra2md.mix").is_some(), "this test must exercise both editions");
        let mut initialization_errors = Vec::new();
        for edition in [Edition::RedAlert2, Edition::YuriAssets] {
            let files = GameFiles::open_for_edition(&dir, edition).unwrap();
            let metadata = CampaignMetadata::load(&files).unwrap();
            // Validate every formal map's country/house/owner graph, rather
            // than assuming the first mission's special houses are universal.
            for side in [CampaignSide::Allied, CampaignSide::Soviet] {
                for stage in 0..metadata.stages(side).len() {
                    let run = metadata.run(side, stage, CampaignDifficulty::Normal).unwrap();
                    let map = files.get(&run.scenario).and_then(Map::parse).unwrap();
                    let init = match ScenarioInit::parse(&files, &map, &run) {
                        Ok(init) => init,
                        Err(error) => { initialization_errors.push(format!("{}: {error}", run.scenario)); continue; }
                    };
                    for owner in map.units.iter().chain(&map.aircraft).map(|o| o.owner.as_str()).chain(map.infantry.iter().map(|o| o.owner.as_str())).chain(map.structures.iter().map(|o| o.owner.as_str())) {
                        init.owner(owner).unwrap_or_else(|error| panic!("{}: {error}", run.scenario));
                    }
                }
                // The default keeps the four first-mission regression fast.
                // The broader installed-asset audit builds every formal world.
                let world_stages = if std::env::var_os("RA2_VALIDATE_ALL_CAMPAIGN_WORLDS").is_some() { metadata.stages(side).len() } else { 1 };
                for stage in 0..world_stages {
                let run = metadata.run(side, stage, CampaignDifficulty::Normal).unwrap();
                let map = files.get(&run.scenario).and_then(Map::parse).unwrap();
                let mut scene = build_scene(&files, &map, &run).unwrap();
                let unit_names: HashSet<_> = scene.unit_types.iter().map(|kind| kind.name.to_ascii_uppercase()).collect();
                let building_names: HashSet<_> = scene.building_types.iter().map(|kind| kind.name.to_ascii_uppercase()).collect();
                let mut world = crate::units::build_world(&files, &mut scene).world;
                let init = scene.scenario.as_ref().unwrap();
                assert_eq!(world.houses.len(), init.houses.len(), "{}", run.scenario);
                assert!(world.damage_fire_rules.enabled, "{} has unsupported original DamageFireTypes metadata", run.scenario);
                world.damage_fire_rules.validate().unwrap();
                assert!(world.houses.iter().any(|house| !house.human && house.ai.as_ref().is_some_and(|brain| brain.campaign.is_some())), "{} has no campaign computer controllers", run.scenario);
                assert!(world.houses[init.local as usize].ai.is_none(), "the player's house acquired an AI controller");
                assert_eq!(world.team_data.team_types.len(), init.definition.team_types.len());
                assert!(world.team_data.triggers.iter().flat_map(|trigger| trigger.team1.into_iter().chain(trigger.team2)).all(|kind| kind < world.team_data.team_types.len()), "{} AI trigger points at a foreign team registry", run.scenario);
                let actual: HashSet<_> = scene.scenario_objects.iter().map(|(source, _)| (source.section, source.id.as_str())).collect();
                let missing: Vec<_> = map.units.iter().filter(|o| unit_names.contains(&o.kind.to_ascii_uppercase())).map(|o| ("Units", o.id.as_str())).chain(map.aircraft.iter().filter(|o| unit_names.contains(&o.kind.to_ascii_uppercase())).map(|o| ("Aircraft", o.id.as_str()))).chain(map.infantry.iter().filter(|o| unit_names.contains(&o.kind.to_ascii_uppercase())).map(|o| ("Infantry", o.id.as_str()))).chain(map.structures.iter().filter(|o| building_names.contains(&o.kind.to_ascii_uppercase())).map(|o| ("Structures", o.id.as_str()))).filter(|key| !actual.contains(key)).collect();
                assert!(missing.is_empty(), "{} lost preplaced objects: {missing:?}", run.scenario);
                assert_eq!(world.units().len(), map.units.iter().chain(&map.aircraft).filter(|o| unit_names.contains(&o.kind.to_ascii_uppercase())).count() + map.infantry.iter().filter(|o| unit_names.contains(&o.kind.to_ascii_uppercase())).count(), "{} added skirmish starting units", run.scenario);
                let unresolved: Vec<_> = map.structures.iter().filter(|o| !building_names.contains(&o.kind.to_ascii_uppercase())).map(|o| (&o.id, &o.kind)).collect();
                if !unresolved.is_empty() { eprintln!("{} has original placement rows with unregistered types: {unresolved:?}", run.scenario); }
                for (placement, actual) in &scene.scenario_objects {
                    if let Aim::Unit(id) = actual {
                        let unit = world.unit(*id).unwrap();
                        assert_eq!(unit.health, (unit.stats.strength.max(0) as i64 * placement.health.clamp(0, 256) as i64 / 256) as i32);
                        assert_eq!(unit.veterancy, placement.veterancy);
                    } else if let Aim::Building(id) = actual {
                        let authored = map.structures.iter().find(|row| row.id == placement.id).unwrap();
                        let building = world.buildings().iter().find(|building| building.id == *id).unwrap();
                        assert_eq!(building.arms.turret_facing(), authored.facing, "{} lost Structures[{}] facing", run.scenario, authored.id);
                        let strength = world.building_types[building.kind as usize].strength;
                        assert_eq!(building.health, (strength as i64 * authored.health.clamp(1, 256) as i64 / 256).max(1) as i32);
                    }
                }
                attach(&mut world, init, &scene.scenario_objects).unwrap();
                let native_houses: Vec<_> = world.mission_data.as_ref().unwrap().houses.iter()
                    .map(|binding| (binding.name.as_str(), binding.house)).collect();
                let authored_houses: Vec<_> = init.definition.houses.iter()
                    .map(|source| (source.name.as_str(), init.owner(&source.name).unwrap())).collect();
                assert_eq!(native_houses, authored_houses, "{} reordered House.Array for runtime Neutral", run.scenario);
                if side == CampaignSide::Soviet && stage == 0 {
                    // Coherent original-executable House reads at frame 0:
                    // game.exe 73288c03..df / gamemd.exe 3e81a617..00.
                    // The complete masks stay equal in the later captured
                    // RA2 0..601 / YR 0..302 samples. These are raw masks in
                    // original House.Array order, including Neutral's slot.
                    let expected: &[u32] = match edition {
                        Edition::RedAlert2 => &[128, 28, 26, 118, 238, 222, 190, 1],
                        Edition::YuriAssets => &[22658, 21121, 61416, 32740, 4099, 32716, 6784, 6211, 32768, 6210, 31724, 704, 0, 24556, 16367, 4352],
                    };
                    let bindings = &world.mission_data.as_ref().unwrap().houses;
                    let raw: Vec<_> = bindings.iter().map(|owner| bindings.iter().enumerate().fold(0u32, |mask, (index, target)| {
                        if world.houses[owner.house as usize].allies & (1u32 << target.house) != 0 { mask | (1u32 << index) } else { mask }
                    })).collect();
                    assert_eq!(raw, expected, "{} differs from native initial raw alliance masks", run.scenario);
                }
                for (house, state) in world.houses.iter().enumerate() {
                    assert_eq!(world.house_controlled_by(house as u8, init.local), state.human, "{} dropped control of a campaign PlayerControl house", run.scenario);
                    assert!(world.allied(house as u8, house as u8), "{} lost self-alliance semantics", run.scenario);
                }
                for _ in 0..150 { world.tick(); }
                let snapshot = world.snapshot();
                for _ in 0..150 { world.tick(); }
                let expected = world.state_hashes();
                let outputs = world.take_mission_outputs();
                for output in &outputs {
                    if let MissionOutput::Camera { .. } = output { eprintln!("{} camera request: {output:?}", run.scenario); }
                }
                if edition == Edition::YuriAssets && side == CampaignSide::Allied && stage == 0 {
                    let home = init.local_position().unwrap();
                    assert!(world.map.revealed_to(init.local, home.0, home.1), "the preplaced American base does not reveal its HomeCell");
                }
                let fired = outputs.iter().filter(|o| matches!(o, MissionOutput::TriggerFired { .. })).count();
                if fired > 1000 {
                    let mut counts = std::collections::BTreeMap::new();
                    for output in &outputs {
                        if let MissionOutput::TriggerFired { trigger_id, .. } = output { *counts.entry(trigger_id.as_str()).or_insert(0usize) += 1; }
                    }
                    let mut counts: Vec<_> = counts.into_iter().collect();
                    counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
                    eprintln!("{} frequent trigger calls: {:?}", run.scenario, &counts[..counts.len().min(10)]);
                }
                assert!(fired > 0, "{} did not run its opening triggers", run.scenario);
                let diagnostics = world.mission_state.as_ref().unwrap().diagnostics.clone();
                world.restore(&snapshot).unwrap();
                for _ in 0..150 { world.tick(); }
                assert_eq!(world.state_hashes(), expected, "{} diverged after restore", run.scenario);
                if let Some(dir) = std::env::var_os("RA2_EXPORT_OUTCOME_FIXTURES").filter(|_| stage == 0) {
                    crate::saves::export_outcome_fixtures(&files, &run, &mut world, std::path::Path::new(&dir));
                }
                eprintln!("campaign {:?}/{}: {} houses, {} placements, {fired} trigger firings; {} explicit behavior gaps in first 300 ticks", edition, run.scenario, init.houses.len(), scene.scenario_objects.len(), diagnostics.len());
                for diagnostic in diagnostics { eprintln!("  tick {}: {:?}", diagnostic.tick, diagnostic.kind); }
                }
            }
        }
        assert!(initialization_errors.is_empty(), "campaign initialization errors: {initialization_errors:?}");
    }
}
