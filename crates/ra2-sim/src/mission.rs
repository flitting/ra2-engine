//! Data-driven campaign triggers. This is an initial, deliberately bounded
//! runtime, not a claim that every original campaign opcode is implemented.
//!
//! Verified against the Steam RA2 executable: event 13 uses one shared timer
//! per trigger, event 47 uses the global logical tick divided by 15, and action
//! 53 resets the relative timer even when its target was already enabled.
//! `native/FINDINGS.md` in the sibling research directory records the evidence.
//! Tag dispatch follows the original sequential RegisterEvent / FireActions
//! chain. Full Logic/House within-frame source ordering remains provisional;
//! its diagnostic remains visible in every mission.

use crate::{World, TICKS_PER_SECOND};
use serde::{Deserialize, Serialize};

/// A map's house name and country name bound to an actual simulation house.
/// A Country is a rules type; it is not the runtime House's display name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseBinding {
    pub name: String,
    pub country: String,
    pub house: u8,
}

/// An original event and its value. `argument` retains the extra YR type
/// argument where present; unsupported kinds are never discarded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDef {
    pub kind: i32,
    pub parameter: i32,
    pub argument: String,
}

/// Original action fields after the opcode, without lossy integer conversion.
/// Slot 0 is the argument type, slot 1 the main parameter and slot 6 the encoded
/// waypoint. For example, slot 1 can be a CSF label, trigger ID or Country index.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDef {
    pub kind: i32,
    pub parameters: [String; 7],
}

impl ActionDef {
    /// Convenient for callers constructing small diagnostic scenarios.
    pub fn with_parameter(kind: i32, parameter: impl Into<String>) -> Self {
        let mut action = Self {
            kind,
            ..Self::default()
        };
        action.parameters[1] = parameter.into();
        action
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDef {
    pub id: String,
    pub country: String,
    pub linked: Option<String>,
    pub enabled: bool,
    /// Easy, normal, hard, in original map order.
    pub difficulty: [bool; 3],
    /// Original TriggerType.MustTransfer: a capturing engineer shares its Tag
    /// with the captured building before raising its own consumption Event48.
    pub must_transfer: bool,
    pub events: Vec<EventDef>,
    pub actions: Vec<ActionDef>,
}

impl Default for TriggerDef {
    fn default() -> Self {
        Self {
            id: String::new(),
            country: String::new(),
            linked: None,
            enabled: true,
            difficulty: [true; 3],
            must_transfer: false,
            events: Vec::new(),
            actions: Vec::new(),
        }
    }
}

/// A tag roots a linked list of trigger types. Definitions not reachable from
/// a tag are not live trigger instances. Persistence 0 expires on the first
/// match, 1 waits for the last attachment, and 2 retains the Tag and event bits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagDef {
    pub id: String,
    pub name: String,
    pub trigger: String,
    pub persistence: u8,
}

/// A placed object's original line identity and its actual simulation identity.
/// The raw initial mission is retained even while its behavior is unsupported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectBinding {
    /// Original INI section (`Units`, `Infantry`, `Aircraft`, `Structures`).
    pub section: String,
    /// Original row key, never the runtime object's numerical ID.
    pub id: String,
    /// Original Tag ID, with `<none>` represented by None.
    pub tag: Option<String>,
    /// The ID returned by add_unit/add_infantry or the actual Building.id.
    pub object: crate::combat::Aim,
    /// Original placement Group. Missing/unspecified is -1.
    pub group: i32,
    pub initial_mission: String,
}

/// Map-only TeamType metadata. The ordinary team, script and task-force rules
/// live in World.team_data; `kind` indexes its team_types without reordering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamDef {
    pub kind: usize,
    /// Original TeamType.House Country name, not the Trigger's owning House.
    pub house_country: String,
    /// Decoded original letter waypoint; None means the original -1 sentinel.
    pub waypoint: Option<i32>,
    pub transport_waypoint: Option<i32>,
    pub tag: Option<String>,
    /// Effective Group: TeamType.Group, or TaskForce.Group when that is -1.
    /// -2 permits any placement group; -1 still denotes a real group value.
    pub group: i32,
    pub loose_recruit: bool,
    /// Preserve the source properties while their remaining behaviors are audited.
    pub properties: Vec<(String, String)>,
}

/// Supported original unit Mission numbers. These are separate from a team's
/// Script opcode and from the trigger Action opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitMission {
    Sleep,
    Guard,
    AreaGuard,
    Hunt,
    Stop,
    Harmless,
}

impl UnitMission {
    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::Sleep,
            5 => Self::Guard,
            11 => Self::AreaGuard,
            15 => Self::Hunt,
            13 => Self::Stop,
            23 => Self::Harmless,
            _ => return None,
        })
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "sleep" => Self::Sleep,
            "guard" => Self::Guard,
            "area guard" => Self::AreaGuard,
            "hunt" => Self::Hunt,
            "stop" => Self::Stop,
            "harmless" => Self::Harmless,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitMissionState {
    pub unit: u32,
    pub mission: UnitMission,
    pub assigned_at: u64,
    pub next_scan: u64,
}

impl Default for TeamDef {
    fn default() -> Self {
        Self {
            kind: 0,
            house_country: String::new(),
            waypoint: None,
            transport_waypoint: None,
            tag: None,
            group: -1,
            loose_recruit: false,
            properties: Vec::new(),
        }
    }
}

/// Original TechnoType identity bound to the simulator's separate type tables.
/// Unit indices cover its combined Vehicle/Infantry/Aircraft table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionTechnoKind { Building(u16), Unit(u16) }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TechnoTypeBinding {
    pub id: String,
    pub kind: MissionTechnoKind,
}

/// Immutable mission rules, attached again before a snapshot is restored.
/// They are deliberately excluded from World snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionData {
    pub id: String,
    /// Original Yuri's Revenge rule branches, never inferred from a unit type.
    pub yuri: bool,
    /// Original House.Array order, with each entry mapped to its runtime ID.
    /// Runtime Neutral may occupy another slot; never sort these by house ID.
    /// For authored Houses this position supplies event44's ArrayIndex;
    /// unbound runtime/synthetic Houses do not inherit that numbering.
    pub houses: Vec<HouseBinding>,
    /// Runtime House ID -> original Edge enum (North/East/South/West/Air =
    /// 0/1/2/3/4; unknown or missing = -1). Ordinary reinforcements use this
    /// only for initial facing; an explicit inside waypoint starts north.
    pub house_edges: Vec<(u8, i32)>,
    /// Zero-based World.unit_types index -> original locomotor CLSID. Retained
    /// to distinguish Tunnel from ordinary ground placement.
    pub unit_locomotors: Vec<(u16, String)>,
    /// Original YR OpenTopped types, by zero-based World.unit_types index.
    /// Closed Full packing must not silently imply passenger combat support.
    pub unit_open_topped: Vec<u16>,
    /// Full Country registration order, including the map's custom countries.
    pub country_names: Vec<String>,
    /// Exact original IDs, independently of UI display names. Retail YR60/61
    /// compare case-sensitive IDs in TechnoType.Array before counting.
    pub techno_types: Vec<TechnoTypeBinding>,
    /// Original DontScore flag; YR alone excludes these types from the House
    /// OwnedBuildingTypes counter used by events32/57. RA2 counts them.
    pub building_dont_score: Vec<u16>,
    pub player_house: u8,
    /// Original IsHumanPlayer, separately from campaign PlayerControl. None
    /// preserves synthetic callers' existing House.human interpretation.
    pub human_player_houses: Option<Vec<u8>>,
    /// Original Rules.AllyReveal. Sight sharing follows source-owner Allies,
    /// independently of whether either House is under player control.
    pub ally_reveal: bool,
    /// Original General.RevealByHeight, default true in the retail ctor.
    pub reveal_by_height: bool,
    /// Runtime source House -> original RadarVisibleTo Country-index mask.
    pub radar_visible_to: Vec<(u8, u32)>,
    pub difficulty: u8,
    pub triggers: Vec<TriggerDef>,
    pub tags: Vec<TagDef>,
    pub local_variables: Vec<(i32, bool)>,
    pub global_variables: Vec<(i32, bool)>,
    /// The fixed local array has 50 entries in RA2 and 100 in YR.
    pub local_variable_limit: i32,
    pub global_variable_limit: i32,
    pub waypoints: Vec<(i32, (i32, i32))>,
    pub objects: Vec<ObjectBinding>,
    pub cell_tags: Vec<((i32, i32), String)>,
    /// Original map TeamType ID -> index in World.team_data.team_types.
    pub team_types: Vec<(String, usize)>,
    pub team_definitions: Vec<TeamDef>,
    pub campaign_ai_rules: crate::ai::CampaignRules,
    pub disabled_ai_triggers: Vec<usize>,
    pub global_ai_triggers: Vec<usize>,
    pub ignore_global_ai_triggers: bool,
    /// Runtime houses whose Country has MultiplayPassive=yes.
    pub passive_houses: Vec<u8>,
    /// Mission rate converted from INI minutes to logical ticks. Defaults are
    /// the original Sleep/Guard/Area Guard/Hunt/Stop/Harmless rates.
    pub unit_mission_rates: Vec<(UnitMission, u32)>,
    /// rules.ini `[General] RevealTriggerRadius`, in cells.
    pub reveal_radius: i32,
    /// Original General.SavourDelay, multiplied by 900 in x87 precision and
    /// truncated to the original signed timer duration.
    pub savour_delay_ticks: i32,
}

impl Default for MissionData {
    fn default() -> Self {
        Self {
            id: String::new(),
            yuri: false,
            houses: Vec::new(),
            house_edges: Vec::new(),
            unit_locomotors: Vec::new(),
            unit_open_topped: Vec::new(),
            country_names: Vec::new(),
            techno_types: Vec::new(),
            building_dont_score: Vec::new(),
            player_house: 0,
            human_player_houses: None,
            ally_reveal: true,
            reveal_by_height: true,
            radar_visible_to: Vec::new(),
            difficulty: 1,
            triggers: Vec::new(),
            tags: Vec::new(),
            local_variables: Vec::new(),
            global_variables: Vec::new(),
            local_variable_limit: 50,
            global_variable_limit: 50,
            waypoints: Vec::new(),
            objects: Vec::new(),
            cell_tags: Vec::new(),
            team_types: Vec::new(),
            team_definitions: Vec::new(),
            campaign_ai_rules: crate::ai::CampaignRules::default(),
            disabled_ai_triggers: Vec::new(),
            global_ai_triggers: Vec::new(),
            ignore_global_ai_triggers: false,
            passive_houses: Vec::new(),
            unit_mission_rates: vec![
                (UnitMission::Sleep, 450),
                (UnitMission::Guard, 27),
                (UnitMission::AreaGuard, 36),
                (UnitMission::Hunt, 14),
                (UnitMission::Stop, 450),
                (UnitMission::Harmless, 450),
            ],
            reveal_radius: 9,
            savour_delay_ticks: 90,
        }
    }
}

/// One timer shared by the trigger's relative-time events, as in RA2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionTimer {
    pub start_tick: u64,
    pub length_ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerState {
    pub id: String,
    pub tag: usize,
    pub enabled: bool,
    pub destroyed: bool,
    pub timer: MissionTimer,
    pub occurred: Vec<bool>,
    /// YR RegisterEvent copies a successful TEvent's nonnull House here even
    /// if another condition is false. FireActions still uses Type.House.
    pub house: Option<u8>,
    pub fire_count: u64,
    /// Number of timer resets, including an Enable of an enabled instance.
    pub reset_generation: u64,
}

/// Original RaiseEvent arguments. `object` is pTagOwner; ordinary damage uses
/// the attacking Techno as separate pSource. Movement/destruction use null.
/// A cell source retains its location through ReplaceTag detachment.
#[derive(Clone, Copy)]
struct MissionEventContext {
    kind: i32,
    object: Option<crate::combat::Aim>,
    source: Option<crate::combat::Aim>,
    cell: Option<(i32, i32)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagState {
    pub id: String,
    /// Index in immutable MissionData.tags, separate from this runtime index.
    pub definition: usize,
    /// Map loading uses GetInstance; a Team constructor always makes a new Tag.
    pub team: Option<u32>,
    pub instantiated: bool,
    /// RaiseEvent announces expiration before the deferred destructor runs;
    /// this is separate from the native Destroyed flag / terminal slot.
    pub pending_delete: bool,
    /// Native Tag.RaiseEvent rejects calls back into its current Trigger chain.
    /// No save can be taken inside the synchronous dispatcher borrow.
    #[serde(skip)]
    pub executing: bool,
    /// Only map-load flag16 registration adds a tag to general Logic polling.
    pub periodic: bool,
    /// Original InstanceCount counts object/cell attachments, not Team pointers.
    /// Keep dead object identities until an actual native detach path occurs.
    pub objects: Vec<crate::combat::Aim>,
    pub cells: Vec<(i32, i32)>,
    pub destroyed: bool,
    pub triggers: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionOutcome {
    Victory { house: u8 },
    Defeat { house: u8 },
}

/// Converts the original INI double without first rounding the multiplication
/// back to 53 bits. The x87 product by integer 900 fits its 64-bit significand.
/// Its __ftol returns INTEGER_INDEFINITE for NaN and signed overflow.
pub fn savour_delay_ticks(minutes: f64) -> i32 {
    let bits = minutes.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    if exponent == 0x7ff { return i32::MIN; }
    let mantissa = (bits & ((1u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1u64 << 52 };
    let shift = (if exponent == 0 { 1 } else { exponent }) - 1023 - 52;
    let product = mantissa as u128 * 900;
    let magnitude = if shift < 0 { product.checked_shr((-shift) as u32).unwrap_or(0) }
        else { match product.checked_shl(shift as u32) { Some(value) if shift < 128 && value.checked_shr(shift as u32) == Some(product) => value, _ => return i32::MIN } };
    if bits >> 63 != 0 {
        if magnitude > (1u128 << 31) { i32::MIN } else { (-(magnitude as i64)) as i32 }
    } else if magnitude > i32::MAX as u128 { i32::MIN } else { magnitude as i32 }
}

/// Original House.Win/Lose share one signed CDTimer. -1 is its stopped-start
/// sentinel; a stopped timer still expires when its stored duration is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeTimer {
    pub start_tick: i32,
    pub length_ticks: i32,
}

impl Default for OutcomeTimer {
    fn default() -> Self { Self { start_tick: -1, length_ticks: 0 } }
}

impl OutcomeTimer {
    fn expired(self, tick: u64) -> bool {
        if self.start_tick == -1 { self.length_ticks == 0 }
        else { (tick as i32).wrapping_sub(self.start_tick) >= self.length_ticks }
    }
}

/// The original pumps window/audio messages at expiry without advancing the
/// logical frame. GUI hosts report its bounded 16 ms wall-clock timer here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomePresentationWait {
    pub outcome: MissionOutcome,
    pub remaining_wall_units: u32,
    pub clear_winner: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionEndgame {
    pub winner: bool,
    pub loser: bool,
    pub timer: OutcomeTimer,
    /// Action15 and the full native AllowWinBlocks counter are not implemented.
    /// Native expiry with its zero initial counter is the supported path.
    pub allow_win_blocks: i32,
    pub presentation_managed: bool,
    pub presentation_wait: Option<OutcomePresentationWait>,
}

impl MissionEndgame {
    /// True only when this call newly sets a result, independently of the
    /// original helper's return value (which reports its resulting flag).
    fn request(&mut self, victory: bool, game_over: bool, tick: u64, length_ticks: i32) -> bool {
        // Lose clears Winner even when a previous GameOver/Loser rejects a
        // new request. Win never overwrites an already losing House.
        if !victory { self.winner = false; }
        if game_over || self.loser || (victory && self.winner) { return false; }
        if victory { self.winner = true; } else { self.loser = true; }
        self.timer = OutcomeTimer { start_tick: tick as i32, length_ticks };
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    SchedulingUnverified,
    CampaignAiUnverified {
        house: u8,
        feature: String,
    },
    UnitMissionTargetingUnverified {
        mission: UnitMission,
    },
    UnitMissionSpecialCase {
        unit: u32,
        mission: UnitMission,
    },
    TeamRecruitmentUnverified {
        team_type: String,
    },
    MissingTeamType {
        team_type: String,
    },
    MissingTagType {
        tag: String,
    },
    TeamCreationFailed {
        team_type: String,
    },
    ReinforcementUnverified {
        team_type: String,
        feature: String,
    },
    ParadropUnverified {
        aircraft: u32,
        feature: String,
    },
    ChangeHouseUnverified {
        object: Option<crate::combat::Aim>,
        feature: String,
    },
    UnsupportedTeamProperty {
        team_type: String,
        property: String,
    },
    UnsupportedScript {
        team: u32,
        team_type: String,
        line: i32,
        opcode: i32,
    },
    MissingScriptWaypoint {
        team: u32,
        waypoint: i32,
    },
    UnsupportedInitialMission {
        object: crate::combat::Aim,
        mission: String,
    },
    UnsupportedPersistence {
        persistence: u8,
    },
    UnsupportedEvent {
        kind: i32,
        index: usize,
    },
    UnsupportedAction {
        kind: i32,
        index: usize,
    },
    InvalidParameter {
        kind: i32,
        index: usize,
    },
    MissingRuntimeTrigger {
        target: String,
    },
    MissingWaypoint {
        waypoint: String,
    },
    MissingCountryHouse {
        country: String,
    },
    RevealGeometryUnverified {
        action: i32,
        feature: String,
    },
    CellEntryUnverified {
        feature: String,
    },
    NonPlayerOutcome {
        // Retained for snapshot layout compatibility. Native actions1/2
        // address CurrentPlayer directly and no longer emit this diagnostic.
        house: u8,
    },
    RandomStreamUnverified {
        feature: String,
    },
}

/// A deterministic, deduplicated report of a first encountered gap. Disabled
/// paths produce no unsupported-opcode reports until they become active.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionDiagnostic {
    pub tick: u64,
    pub trigger_id: Option<String>,
    pub kind: DiagnosticKind,
    pub message: String,
}

/// Presentation and diagnostic output has its own queue. Reading combat
/// events cannot consume a mission's text, speech, camera or objective result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionOutput {
    Text {
        label: String,
    },
    Sound {
        name: String,
    },
    Speech {
        name: String,
    },
    Reveal {
        waypoint: i32,
        cell: (i32, i32),
    },
    Radar {
        kind: i32,
        waypoint: i32,
        cell: (i32, i32),
    },
    Camera {
        speed: i32,
        waypoint: i32,
        cell: (i32, i32),
    },
    TriggerFired {
        tick: u64,
        trigger_id: String,
        tag_id: String,
    },
    Outcome(MissionOutcome),
    Diagnostic(MissionDiagnostic),
}

/// All mutable mission state, including output not yet consumed by the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionState {
    pub id: String,
    pub rules_hash: u64,
    pub team_rules_hash: u64,
    pub attached_at: u64,
    pub tags: Vec<TagState>,
    pub triggers: Vec<TriggerState>,
    /// YR TEvent nodes belong to TriggerType, shared by all of its runtime
    /// Trigger instances. EnteredBy updates the shared definition-node House.
    pub event_houses: Vec<(String, Vec<Option<u8>>)>,
    /// Native enemy/anger belong to every House, including a player with no AI.
    pub diplomacy: Vec<crate::diplomacy::HouseDiplomacy>,
    pub local_variables: Vec<(i32, bool)>,
    pub global_variables: Vec<(i32, bool)>,
    pub outcome: Option<MissionOutcome>,
    pub endgame: MissionEndgame,
    /// Scripted input lock only; AI, mission scripts and logical ticks continue.
    pub input_locked: bool,
    pub diagnostics: Vec<MissionDiagnostic>,
    pub outputs: Vec<MissionOutput>,
    pub unit_missions: Vec<UnitMissionState>,
    /// AddMember writes the team's effective Group to the unit and release
    /// keeps it. Keep the mutable value separate from placement metadata.
    pub unit_groups: Vec<(u32, i32)>,
    /// Runtime TeamType.Autocreate writes made by successful AI selection.
    /// Static team_data remains immutable so saves can verify source rules.
    pub autocreate_team_types: Vec<usize>,
    /// The reinforcement constructor supplies (5,0) to a missing/empty shared
    /// script. Keep that shared runtime write out of immutable team_data.
    pub default_script_overrides: Vec<usize>,
    /// Registration occurs once at lethal damage or an audited consumption,
    /// before disappearance. A crashing aircraft's later physical removal
    /// must not raise it again.
    pub destructions: Vec<crate::combat::Aim>,
}

impl MissionData {
    fn rules_hash(&self) -> u64 {
        // No HashMap iteration or platform-sized integer bytes enter this hash.
        bincode::serialize(self)
            .expect("serializing mission rules")
            .iter()
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
                (hash ^ *byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    fn trigger(&self, id: &str) -> Option<&TriggerDef> {
        self.triggers
            .iter()
            .find(|trigger| trigger.id.eq_ignore_ascii_case(id))
    }

    fn difficulty_allows(&self, trigger: &TriggerDef) -> bool {
        trigger.difficulty[self.difficulty as usize]
    }

    fn country_house(&self, country: &str) -> Option<u8> {
        self.houses
            .iter()
            .filter(|binding| binding.country.eq_ignore_ascii_case(country))
            .map(|binding| binding.house)
            .next()
    }

    fn tag_attach_flags(&self, tag: &TagDef) -> u8 {
        let mut flags = 0;
        let mut current = self.trigger(&tag.trigger);
        while let Some(definition) = current {
            for event in &definition.events { flags |= event_attach_flags(event.kind, self.yuri); }
            current = definition.linked.as_deref().and_then(|id| self.trigger(id));
        }
        flags
    }
}

impl MissionState {
    fn activate_tag(&mut self, data: &MissionData, tag_index: usize, tick: u64) -> Result<(), String> {
        if self.tags[tag_index].instantiated { return Ok(()); }
        let tag = &data.tags[self.tags[tag_index].definition];
        let mut chain = Vec::new();
        let mut current = Some(tag.trigger.as_str());
        let mut instances = Vec::new();
        while let Some(id) = current {
            let definition = data.trigger(id).ok_or_else(|| format!("tag {:?} references missing trigger {id:?}", tag.id))?;
            if chain.contains(&definition.id) { return Err(format!("cyclic mission trigger chain at {:?}", definition.id)); }
            chain.push(definition.id.clone());
            let index = self.triggers.len();
            instances.push(index);
            self.triggers.push(TriggerState {
                id: definition.id.clone(), tag: tag_index,
                enabled: definition.enabled && data.difficulty_allows(definition),
                destroyed: false,
                timer: MissionTimer { start_tick: tick, length_ticks: 0 },
                occurred: vec![false; definition.events.len()], house: None, fire_count: 0, reset_generation: 0,
            });
            self.reset(index, definition, tick);
            current = definition.linked.as_deref().filter(|id| !id.is_empty() && !id.eq_ignore_ascii_case("<none>"));
        }
        // Original Tag constructor inserts each new runtime Trigger at its
        // list head while visiting TriggerType.NextTrigger in source order.
        instances.reverse();
        self.tags[tag_index].triggers = instances;
        self.tags[tag_index].instantiated = true;
        Ok(())
    }

    fn new_team_tag(&mut self, data: &MissionData, definition: usize, team: u32, tick: u64) -> Result<usize, String> {
        let index = self.tags.len();
        self.tags.push(TagState { id: data.tags[definition].id.clone(), definition, team: Some(team), instantiated: false,
            pending_delete: false, executing: false, periodic: false, objects: Vec::new(), cells: Vec::new(), destroyed: false, triggers: Vec::new() });
        self.activate_tag(data, index, tick)?;
        Ok(index)
    }

    fn object_tag(&self, object: crate::combat::Aim) -> Option<usize> {
        self.tags.iter().position(|tag| tag.instantiated && !tag.destroyed && !tag.pending_delete && tag.objects.contains(&object))
    }

    fn cell_tag(&self, cell: (i32, i32)) -> Option<usize> {
        self.tags.iter().position(|tag| tag.instantiated && !tag.destroyed && !tag.pending_delete && tag.cells.contains(&cell))
    }

    fn attach_object_tag(&mut self, object: crate::combat::Aim, tag_index: Option<usize>) {
        // Native rebinding decrements the former count even for the same Tag,
        // then attaches/increments the new one. Identity lists preserve this.
        for tag in &mut self.tags { tag.objects.retain(|other| *other != object); }
        if let Some(tag) = tag_index { self.tags[tag].objects.push(object); }
    }

    fn tag_refs(&self, tag: usize) -> usize { self.tags[tag].objects.len() + self.tags[tag].cells.len() }

    fn destroy_tag(&mut self, tag: usize) {
        self.tags[tag].destroyed = true;
        for &trigger in &self.tags[tag].triggers { self.triggers[trigger].destroyed = true; }
    }

    /// A variable not explicitly registered starts clear.
    pub fn local(&self, id: i32) -> bool {
        variable(&self.local_variables, id)
    }

    pub fn global(&self, id: i32) -> bool {
        variable(&self.global_variables, id)
    }

    fn diagnose(&mut self, tick: u64, trigger: Option<&str>, kind: DiagnosticKind, message: &str) {
        if self.diagnostics.iter().any(|diagnostic| {
            diagnostic.trigger_id.as_deref() == trigger && diagnostic.kind == kind
        }) {
            return;
        }
        let diagnostic = MissionDiagnostic {
            tick,
            trigger_id: trigger.map(str::to_owned),
            kind,
            message: message.into(),
        };
        self.diagnostics.push(diagnostic.clone());
        self.outputs.push(MissionOutput::Diagnostic(diagnostic));
    }

    fn reset(&mut self, index: usize, definition: &TriggerDef, tick: u64) {
        self.triggers[index].reset_generation =
            self.triggers[index].reset_generation.wrapping_add(1);
        for (event_index, event) in definition.events.iter().enumerate() {
            if event.kind == 13 {
                // The original owns one timer, so the last relative timer in
                // definition order wins when a map supplies more than one.
                self.triggers[index].timer = MissionTimer {
                    start_tick: tick,
                    length_ticks: event.parameter.max(0) as u64 * TICKS_PER_SECOND as u64,
                };
                self.triggers[index].occurred[event_index] = false;
            }
        }
    }
}

/// Original TEvent.GetAttachType tables: object/cell, map, House and general
/// periodic categories. RA2 and YR differ for their added event numbers.
fn event_attach_flags(kind: i32, yuri: bool) -> u8 {
    const RA2: [u8; 64] = [3,3,2,8,3,8,2,2,31,8,8,8,8,16,16,8,8,8,8,8,8,8,8,16,5,1,1,16,16,2,8,1,8,2,2,2,16,16,2,2,2,2,2,2,2,16,16,16,2,2,16,16,8,1,1,8,8,8,0,0,0,0,0,0];
    const YR: [u8; 64] = [3,3,2,8,3,8,2,2,31,8,8,8,8,16,16,8,8,8,8,8,8,8,8,16,5,1,1,16,16,2,8,1,8,2,2,2,16,16,2,2,2,2,2,2,2,16,16,16,2,2,16,16,8,1,1,8,8,8,8,1,16,16,0,0];
    usize::try_from(kind).ok().and_then(|index| if yuri { YR.get(index) } else { RA2.get(index) }).copied().unwrap_or(0)
}

/// The original House tail captures Count once, then reads each decreasing
/// index from the live vector. A callback deleting a lower index can therefore
/// make it visit the same Tag again. Never snapshot IDs or deduplicate visits.
fn visit_house_tags_live<T, V>(frame: u64, state: &mut T, count: usize, mut get: impl FnMut(&T, usize) -> Option<V>, mut dispatch: impl FnMut(&mut T, V)) -> bool {
    if frame % 8 != 0 { return true; }
    for index in (0..count).rev() {
        let Some(tag) = get(state, index) else { return false; };
        dispatch(state, tag);
    }
    true
}

/// Original accepted Object.ReceiveDamage tail. Both calls re-read the live
/// object's state, so6 may detach or replace the Tag or destroy its victim.
/// The caller establishes that the full damage path reached this tail.
fn visit_ordinary_damage_notifications<T, Tag: Copy>(context: &mut T, damage_state: i32, has_source: bool,
    is_alive: impl Fn(&T) -> bool, get_tag: impl Fn(&T) -> Option<Tag>, mut raise: impl FnMut(&mut T, Tag, i32)) {
    if damage_state == 4 || !has_source || !is_alive(context) { return; }
    if let Some(tag) = get_tag(context) { raise(context, tag, 6); }
    if !is_alive(context) { return; }
    if let Some(tag) = get_tag(context) { raise(context, tag, 44); }
}

/// Original YR House.PowerPercent >= 1, including its signed raw-counter
/// boundaries. A nonpositive drain is always full under the original helper;
/// using only !low_power would change negative-drain behavior.
fn native_power_full(output: i32, drain: i32) -> bool { drain <= 0 || output >= drain }

fn variable(variables: &[(i32, bool)], id: i32) -> bool {
    variables
        .iter()
        .find(|(key, _)| *key == id)
        .is_some_and(|(_, value)| *value)
}

fn set_variable(variables: &mut Vec<(i32, bool)>, id: i32, value: bool) -> bool {
    if let Some((_, existing)) = variables.iter_mut().find(|(key, _)| *key == id) {
        if *existing == value {
            return false;
        }
        *existing = value;
    } else if value {
        variables.push((id, value));
    } else {
        return false;
    }
    true
}

fn assign_mission_variable(data: &MissionData, state: &mut MissionState, now: u64, global: bool, id: i32, value: bool) {
    let changed = if global { set_variable(&mut state.global_variables, id, value) }
        else { set_variable(&mut state.local_variables, id, value) };
    if !changed { return; }
    // Trigger actions and script39/40 call the same native Scenario setter.
    // A changed value resets observers even when they are disabled; assigning
    // the existing value does not restart their shared relative timer.
    for target in 0..state.triggers.len() {
        if state.triggers[target].destroyed { continue; }
        let definition = data.trigger(&state.triggers[target].id).expect("validated variable observer");
        if definition.events.iter().any(|event| event.parameter == id && if global { matches!(event.kind, 27 | 28) } else { matches!(event.kind, 36 | 37) }) {
            state.reset(target, definition, now);
        }
    }
}

fn unique_names<'a>(names: impl Iterator<Item = &'a str>, what: &str) -> Result<(), String> {
    let mut seen = Vec::new();
    for name in names {
        if name.is_empty()
            || seen
                .iter()
                .any(|previous: &&str| previous.eq_ignore_ascii_case(name))
        {
            return Err(format!("empty or duplicate mission {what}: {name:?}"));
        }
        seen.push(name);
    }
    Ok(())
}

impl World {
    /// Whether campaign action46 or the blocking outcome audio wait locks the
    /// player's input. This does not gate World::order: AI and script-issued
    /// orders must keep running during action46's logical simulation.
    pub fn mission_input_locked(&self) -> bool {
        self.mission_state.as_ref().is_some_and(|state| state.input_locked || state.endgame.presentation_wait.is_some())
    }

    /// Script39/40 share Scenario.SetLocal with trigger actions56/57. The
    /// caller still completes its script step if the native ID is out of range.
    pub(crate) fn set_campaign_script_local(&mut self, id: i32, value: bool) -> bool {
        let (Some(data), Some(state)) = (&self.mission_data, self.mission_state.as_mut()) else { return false; };
        if !(0..data.local_variable_limit).contains(&id) { return false; }
        assign_mission_variable(data, state, self.tick, false, id, value);
        true
    }

    fn mission_team_rules_hash(&self) -> u64 {
        bincode::serialize(&(
            &self.team_data.team_types,
            &self.team_data.task_forces,
            &self.team_data.scripts,
            &self.team_data.triggers,
            &self.team_data.fill_earliest_probability,
            self.damage_fire_rules_fingerprint(),
        ))
        .expect("serializing mission team rules")
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ *byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// Attach mission rules to an already initialized campaign world. Validation
    /// is atomic: on an error neither an existing mission nor end checks change.
    pub fn attach_mission(&mut self, data: MissionData) -> Result<(), String> {
        if data.id.is_empty() {
            return Err("a mission needs a stable id".into());
        }
        if data.difficulty > 2 {
            return Err("mission difficulty must be 0, 1 or 2".into());
        }
        if data.player_house as usize >= self.houses.len() {
            return Err(format!(
                "mission player house {} is not initialized",
                data.player_house
            ));
        }
        for binding in &data.techno_types {
            let valid = match binding.kind {
                MissionTechnoKind::Building(index) => (index as usize) < self.building_types.len(),
                MissionTechnoKind::Unit(index) => (index as usize) < self.unit_types.len(),
            };
            if binding.id.is_empty() || !valid { return Err(format!("invalid mission TechnoType binding {:?}", binding.id)); }
        }
        if data.building_dont_score.iter().any(|&index| index as usize >= self.building_types.len()) {
            return Err("mission DontScore binding has an unknown BuildingType index".into());
        }
        if data.human_player_houses.as_ref().is_some_and(|houses| houses.iter().any(|&house| house as usize >= self.houses.len())) {
            return Err("native human-player House metadata references an uninitialized House".into());
        }
        unique_names(
            data.houses.iter().map(|binding| binding.name.as_str()),
            "house",
        )?;
        unique_names(
            data.triggers.iter().map(|trigger| trigger.id.as_str()),
            "trigger",
        )?;
        unique_names(data.tags.iter().map(|tag| tag.id.as_str()), "tag")?;
        for (index, binding) in data.houses.iter().enumerate() {
            if binding.house as usize >= self.houses.len() || binding.house >= 32 {
                return Err(format!(
                    "mission house {:?} has invalid runtime house {}",
                    binding.name, binding.house
                ));
            }
            if data.houses[..index]
                .iter()
                .any(|other| other.house == binding.house)
            {
                return Err(format!("duplicate mission runtime house {}", binding.house));
            }
        }
        for (label, variables, limit) in [
            ("local", &data.local_variables, data.local_variable_limit),
            ("global", &data.global_variables, data.global_variable_limit),
        ] {
            if limit <= 0 {
                return Err(format!("invalid {label} mission variable limit {limit}"));
            }
            for (index, (id, _)) in variables.iter().enumerate() {
                if *id < 0
                    || *id >= limit
                    || variables[..index].iter().any(|(other, _)| other == id)
                {
                    return Err(format!(
                        "invalid or duplicate {label} mission variable {id}"
                    ));
                }
            }
        }
        for (index, (id, cell)) in data.waypoints.iter().enumerate() {
            if *id < 0
                || data.waypoints[..index].iter().any(|(other, _)| other == id)
                || self.map.index(cell.0, cell.1).is_none()
            {
                return Err(format!(
                    "invalid or duplicate mission waypoint {id} at {cell:?}"
                ));
            }
        }
        let mut state = MissionState {
            id: data.id.clone(),
            rules_hash: data.rules_hash(),
            team_rules_hash: self.mission_team_rules_hash(),
            attached_at: self.tick,
            tags: Vec::new(),
            triggers: Vec::new(),
            event_houses: data.triggers.iter().map(|definition| (definition.id.clone(), vec![None; definition.events.len()])).collect(),
            diplomacy: data.houses.iter().map(|owner| crate::diplomacy::HouseDiplomacy {
                house: owner.house, enemy_index: -1,
                anger: data.houses.iter().filter(|other| other.house != owner.house).map(|other| crate::diplomacy::AngerNode { house: other.house, score: 0 }).collect(),
            }).collect(),
            local_variables: data.local_variables.clone(),
            global_variables: data.global_variables.clone(),
            outcome: None,
            endgame: MissionEndgame::default(),
            input_locked: false,
            diagnostics: Vec::new(),
            outputs: Vec::new(),
            unit_missions: Vec::new(),
            unit_groups: data
                .objects
                .iter()
                .filter_map(|binding| match binding.object {
                    crate::combat::Aim::Unit(id) => Some((id, binding.group)),
                    _ => None,
                })
                .collect(),
            autocreate_team_types: Vec::new(),
            default_script_overrides: Vec::new(),
            destructions: Vec::new(),
        };
        for (tag_index, tag) in data.tags.iter().enumerate() {
            let mut flags = 0;
            let mut chain = Vec::new();
            let mut current = Some(tag.trigger.as_str());
            while let Some(id) = current {
                let definition = data.trigger(id).ok_or_else(|| format!("tag {:?} references missing trigger {id:?}", tag.id))?;
                if chain.contains(&definition.id) { return Err(format!("cyclic mission trigger chain at {:?}", definition.id)); }
                chain.push(definition.id.clone());
                for event in &definition.events { flags |= event_attach_flags(event.kind, data.yuri); }
                current = definition.linked.as_deref().filter(|id| !id.is_empty() && !id.eq_ignore_ascii_case("<none>"));
            }
            let attached = data.objects.iter().any(|object| object.tag.as_ref().is_some_and(|id| id.eq_ignore_ascii_case(&tag.id)))
                || data.cell_tags.iter().any(|(_, id)| id.eq_ignore_ascii_case(&tag.id));
            let periodic = flags & 0x10 != 0;
            state.tags.push(TagState { id: tag.id.clone(), definition: tag_index, team: None,
                instantiated: false, pending_delete: false, executing: false, periodic, objects: Vec::new(), cells: Vec::new(), destroyed: false, triggers: Vec::new() });
            // Cell/map (4), House (8) and general Logic (16) have distinct
            // native registration vectors. Only the last receives timer13.
            if flags & 0x1c != 0 || attached { state.activate_tag(&data, tag_index, self.tick)?; }
        }
        for object in &data.objects {
            if let Some(tag) = &object.tag {
                if let Some(index) = data.tags.iter().position(|definition| definition.id.eq_ignore_ascii_case(tag)) {
                    state.attach_object_tag(object.object, Some(index));
                } else { state.diagnose(self.tick, None, DiagnosticKind::MissingTagType { tag: tag.clone() }, "The placed object's raw Tag reference does not resolve to a TagType."); }
            }
        }
        for &(cell, ref tag) in &data.cell_tags {
            if let Some(index) = data.tags.iter().position(|definition| definition.id.eq_ignore_ascii_case(tag)) {
                for instance in &mut state.tags { instance.cells.retain(|&other| other != cell); }
                state.tags[index].cells.push(cell);
            } else { state.diagnose(self.tick, None, DiagnosticKind::MissingTagType { tag: tag.clone() }, "The CellTag reference does not resolve to a TagType."); }
        }
        state.diagnose(self.tick, None, DiagnosticKind::SchedulingUnverified,
            "Tags register and fire each linked Trigger sequentially, with immediate Tag expiration after the chain. House sources run on each eighth frame with live reverse RelatedTags reads. Ordinary positive nonlethal damage emits6 then re-reads life/Tag for44; complete native damage normalization, IsAlive/limbo transitions, combat subclass phases, Logic/House scheduling and cross-vector/special notifications remain unverified.");
        if !data.cell_tags.is_empty() {
            state.diagnose(self.tick, None, DiagnosticKind::CellEntryUnverified { feature: "SpecialLocomotorAndCurveSplice".into() },
                "EnteredBy runs synchronously at ordinary Walk/Drive completion, teleport and falling landing, with native Country matching and Tag detachment. Successful drive-curve splicing, Jumpjet landing, transport boarding, refused building-radio requests and all bridge flag variants are not fully reproduced; placement and stationary cells are not polled.");
            if data.yuri {
                state.diagnose(self.tick, None, DiagnosticKind::CellEntryUnverified { feature: "AirborneFootUpdatePhase".into() },
                    "YR's airborne CellTag source raises59 each sixteenth frame for placed Foot at relative height>=208. It runs at this engine's per-Foot movement tail; complete original Foot.Update/locomotor scheduling and special Mark/limbo transitions remain unverified. Only flat, mapped ordinary placements participate; unsupported geometry or placement state is diagnosed.");
            }
        }
        for (house, owner) in self.houses.iter().enumerate() {
            if owner
                .ai
                .as_ref()
                .is_some_and(|brain| brain.campaign.is_some())
            {
                state.diagnose(self.tick, None, DiagnosticKind::CampaignAiUnverified { house: house as u8, feature: "HouseAIExecution".into() },
                    "Campaign production switches, edition-specific selection timing, authored Base nodes and AI-trigger gates run. Complete original production eligibility, legacy autocreate, defence response and within-frame AI ordering still require differential verification.");
            }
        }
        if self.campaign_random.is_some() {
            state.diagnose(self.tick, None, DiagnosticKind::RandomStreamUnverified { feature: "ScenarioConsumptionOrder".into() },
                "The original 250-word Scenario.Random is saved and used by audited AI selection/delay, reinforcement edge and Infantry subposition requests. Original setup/constructor draws, Infantry idle/scatter and other unaudited consumers still use existing paths; complete original stream consumption and frame order are not yet reproduced.");
        }
        if self.units.iter().any(|unit| unit.stats.infantry) {
            state.diagnose(self.tick, None, DiagnosticKind::RandomStreamUnverified { feature: "InfantryIdleAndFearDriver".into() },
                "Campaign Infantry still use the existing idle/fear driver. Original due Guard/Hunt callbacks, Scenario wait/action/COW/turn requests, Global VoiceIdle and actual animation/Scatter completion are not integrated. The separately verified pure idle arithmetic is not runtime consumption parity; Sleep's native independent Fraidycat escape is also still pending.");
        }
        for binding in &data.objects {
            if !binding.initial_mission.is_empty()
                && UnitMission::from_name(&binding.initial_mission).is_none()
            {
                state.diagnose(self.tick, None, DiagnosticKind::UnsupportedInitialMission { object: binding.object, mission: binding.initial_mission.clone() },
                    "The placed object's initial mission is retained but its original execution has not yet been implemented.");
            }
        }
        let views = self.diplomacy_views(&data);
        self.apply_diplomacy_projection(&data, &state, &views);
        self.mission_data = Some(data);
        self.mission_state = Some(state);
        // Campaign reveal gates now target one CurrentPlayer and may include
        // allied/RadarVisibleTo sources. A previous per-House sight cache
        // cannot suppress those sources' first look for the new recipient.
        for unit in &mut self.units { unit.looked = None; }
        for building in &mut self.buildings { building.looked = false; }
        for (house, owner) in self.houses.iter_mut().enumerate() {
            if let Some(brain) = owner
                .ai
                .as_mut()
                .filter(|brain| brain.campaign.is_some() && !brain.timer_armed)
            {
                let index = brain
                    .original_house_index
                    .unwrap_or(house.saturating_sub(1) as u32);
                brain.timer_armed = true;
                brain.team_timer = self.tick
                    + self.team_data.team_delays[brain.difficulty.min(2)] as u64
                    + 175 * index as u64;
            }
        }
        let initial: Vec<_> = self
            .mission_data
            .as_ref()
            .unwrap()
            .objects
            .iter()
            .filter_map(|binding| {
                match (
                    binding.object,
                    UnitMission::from_name(&binding.initial_mission),
                ) {
                    (crate::combat::Aim::Unit(id), Some(mission)) => Some((id, mission)),
                    _ => None,
                }
            })
            .collect();
        for (id, mission) in initial {
            self.assign_unit_mission(id, mission);
        }
        // Campaign objectives, rather than skirmish base elimination, decide
        // the result. Some missions start with no player base at all.
        self.end_checks = false;
        Ok(())
    }

    /// GUI hosts enable the original bounded audio-message wait. Headless
    /// simulation leaves it disabled and finishes immediately at timer expiry.
    pub fn set_mission_presentation_managed(&mut self, managed: bool) {
        if let Some(state) = self.mission_state.as_mut() {
            state.endgame.presentation_managed = managed;
        }
        if !managed { self.advance_mission_presentation_time(0, false); }
    }

    pub fn mission_presentation_waiting(&self) -> bool {
        self.mission_state.as_ref().is_some_and(|state| state.endgame.presentation_wait.is_some())
    }

    /// Reports elapsed GetTickCount >> 4 units (16 ms) since the previous
    /// presentation poll. Calling this never advances the logical frame.
    /// Returns true only when this call completes the pending mission outcome.
    pub fn advance_mission_presentation_time(&mut self, elapsed_units: u32, audio_busy: bool) -> bool {
        let Some(state) = self.mission_state.as_mut() else { return false; };
        let Some(wait) = state.endgame.presentation_wait.as_mut() else { return false; };
        wait.remaining_wall_units = wait.remaining_wall_units.saturating_sub(elapsed_units);
        if audio_busy && wait.remaining_wall_units > 0 { return false; }
        self.finish_mission_outcome();
        true
    }

    fn finish_mission_outcome(&mut self) {
        let state = self.mission_state.as_mut().expect("mission outcome has state");
        let wait = state.endgame.presentation_wait.take().expect("mission outcome is waiting");
        if wait.clear_winner { state.endgame.winner = false; }
        state.outcome = Some(wait.outcome);
        state.outputs.push(MissionOutput::Outcome(wait.outcome));
        self.over = Some(match wait.outcome {
            MissionOutcome::Victory { house } => vec![house],
            MissionOutcome::Defeat { .. } => Vec::new(),
        });
    }

    pub(crate) fn mission_simulation_stopped(&self) -> bool {
        self.mission_state.as_ref().is_some_and(|state| state.outcome.is_some() || state.endgame.presentation_wait.is_some())
    }

    /// Bounded House.AI lifecycle for the CurrentPlayer targeted by actions1/2.
    /// The surrounding Logic/House scheduler retains its explicit diagnostic.
    pub(crate) fn run_mission_endgame(&mut self) -> bool {
        let (Some(data), Some(state)) = (&self.mission_data, &mut self.mission_state) else { return false; };
        if state.outcome.is_some() || state.endgame.presentation_wait.is_some() { return true; }
        if !state.endgame.timer.expired(self.tick) { return false; }
        let win = state.endgame.winner && state.endgame.allow_win_blocks <= 0;
        let lose = state.endgame.loser;
        if !win && !lose { return false; }
        let house = data.player_house;
        // The original campaign House.AI gate accepts either IsHumanPlayer or
        // PlayerControl. World.House.human stores that union for campaign input.
        let human = self.houses[house as usize].human || data.human_player_houses.as_ref().is_some_and(|houses| houses.contains(&house));
        let victory = if lose { !human } else { human };
        state.endgame.presentation_wait = Some(OutcomePresentationWait {
            outcome: if victory { MissionOutcome::Victory { house } } else { MissionOutcome::Defeat { house } },
            remaining_wall_units: 120,
            clear_winner: win,
        });
        if !state.endgame.presentation_managed { self.advance_mission_presentation_time(0, false); }
        true
    }

    pub fn take_mission_outputs(&mut self) -> Vec<MissionOutput> {
        self.mission_state
            .as_mut()
            .map_or_else(Vec::new, |state| std::mem::take(&mut state.outputs))
    }

    /// Saved diplomacy also exists for a player House without an AI Brain.
    pub fn mission_diplomacy(&self, house: u8) -> Option<&crate::diplomacy::HouseDiplomacy> {
        self.mission_state.as_ref()?.diplomacy.iter().find(|state| state.house == house)
    }

    /// Resolve the authoritative original EnemyHouseIndex to a runtime slot.
    /// No current-player or nearest-base target is supplied when it is absent.
    pub(crate) fn enemy_house(&self, house: u8) -> Option<u8> {
        if let Some(data) = self.mission_data.as_ref() {
            let index = usize::try_from(self.mission_diplomacy(house)?.enemy_index).ok()?;
            return data.houses.get(index).map(|binding| binding.house);
        }
        self.houses.get(house as usize)?.ai.as_ref()?.enemy
    }

    fn diplomacy_views(&self, data: &MissionData) -> Vec<crate::diplomacy::HouseView> {
        data.houses.iter().enumerate().map(|(index, binding)| {
            let owner = &self.houses[binding.house as usize];
            let allies = data.houses.iter().enumerate().fold(0u32, |mask, (index, target)| {
                if owner.allies & (1u32 << target.house) != 0 { mask | (1u32 << index) } else { mask }
            });
            crate::diplomacy::HouseView { house: binding.house, array_index: index as i32, country_index: owner.country as i32,
                allies, defeated: owner.defeated, passive: data.passive_houses.contains(&binding.house) }
        }).collect()
    }

    fn apply_diplomacy_projection(&mut self, data: &MissionData, state: &MissionState, views: &[crate::diplomacy::HouseView]) {
        let house_count = self.houses.len();
        for view in views {
            let owner = &mut self.houses[view.house as usize];
            for (index, target) in data.houses.iter().enumerate() {
                let runtime_bit = 1u32 << target.house;
                if view.allies & (1u32 << index) != 0 { owner.allies |= runtime_bit; }
                else { owner.allies &= !runtime_bit; }
            }
            let diplomacy = state.diplomacy.iter().find(|entry| entry.house == view.house).expect("validated diplomacy projection");
            if let Some(brain) = owner.ai.as_mut() {
                brain.enemy = usize::try_from(diplomacy.enemy_index).ok().and_then(|index| data.houses.get(index)).map(|entry| entry.house);
                brain.anger = (0..house_count).map(|house| diplomacy.anger.iter().find(|node| node.house as usize == house).map_or(0, |node| node.score)).collect();
            }
        }
    }

    /// Existing combat/Expert callers retain their score and eligibility. Only
    /// the campaign sink adopts original node order, wrapping and stored flags.
    pub(crate) fn adjust_campaign_anger(&mut self, house: u8, delta: i32, target: u8) -> bool {
        let Some(data) = self.mission_data.take() else { return false; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return true; };
        let views = self.diplomacy_views(&data);
        if views.iter().any(|view| view.house == house) && views.iter().any(|view| view.house == target) {
            crate::diplomacy::adjust_anger(&mut state.diplomacy, &views, house, delta, target);
            self.apply_diplomacy_projection(&data, &state, &views);
            state.diagnose(self.tick, None, DiagnosticKind::CampaignAiUnverified { house, feature: "CampaignAngerScoreSource".into() },
                "Campaign AdjustAnger preserves native pointer-node order, wrapping scores and stored defeated/allied flags. Existing damage-score and Expert reset inputs are retained; their complete native score calculation, decay and call scheduling remain unverified.");
        } else {
            state.diagnose(self.tick, None, DiagnosticKind::CampaignAiUnverified { house, feature: "UnboundDiplomacyHouse".into() },
                "AdjustAnger references a runtime House outside the original authored registry. No node or enemy is invented for synthetic/dynamically created Houses.");
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
        true
    }

    fn mission_diplomacy_action(&mut self, data: &MissionData, state: &mut MissionState, trigger: &TriggerDef, runtime: usize, index: usize, action: &ActionDef) -> bool {
        if crate::diplomacy::atoi32(&action.parameters[0]) != 0 {
            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::UnsupportedAction { kind: action.kind, index },
                "Diplomacy requires the audited numeric Type0 Action.Value reader. Another parameter type is not interpreted as an invented Country integer.");
            return false;
        }
        let raw = crate::diplomacy::atoi32(&action.parameters[1]);
        if !data.yuri && raw == -1 { return true; } // Actual RA2 Execute returns before lookup/calls.
        let Some(receiver) = data.country_house(&trigger.country) else {
            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingCountryHouse { country: trigger.country.clone() },
                "Diplomacy uses FireActions' static TriggerType Country House, which is unresolved. The runtime Trigger.House is only YR's explicit8997 target.");
            return false;
        };
        let mut views = self.diplomacy_views(data);
        let resolution = crate::diplomacy::resolve_house(&views, data.yuri, raw, state.triggers.get(runtime).map(|trigger| trigger.house), None);
        let target = match resolution {
            crate::diplomacy::Resolution::House(target) if views.iter().any(|view| view.house == target) => target,
            _ => {
                let message = if resolution == crate::diplomacy::Resolution::UnboundMultiplayer {
                    "YR diplomacy references an MP slot4475..4482 without the original Scenario slot-to-House.Array mapping. It is not treated as a Country or current player."
                } else if data.yuri {
                    "The original YR diplomacy target resolver returned null or an unbound House, so neither directional helper runs.8997 is the runtime Trigger.House sentinel;9000 remains an ordinary Country value."
                } else {
                    "This RA2 diplomacy raw Country has no matching House. Native Execute would dereference null; the host bounds that unsafe path without a fallback or fabricated successful action."
                };
                state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::UnsupportedAction { kind: action.kind, index }, message);
                return false;
            }
        };
        if action.kind == 37 {
            crate::diplomacy::make_ally(&mut state.diplomacy, &mut views, receiver, target);
            crate::diplomacy::make_ally(&mut state.diplomacy, &mut views, target, receiver);
        } else {
            crate::diplomacy::make_enemy(&mut state.diplomacy, &mut views, receiver, target);
            crate::diplomacy::make_enemy(&mut state.diplomacy, &mut views, target, receiver);
        }
        self.apply_diplomacy_projection(data, state, &views);
        state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::CampaignAiUnverified { house: receiver, feature: "DiplomacyWorldEffects".into() },
            "Runtime campaign diplomacy state follows the original two directional helper calls and anger/alliance order. Occupied-world threat/target cancellation, visibility, presentation, ScenarioInit nonzero bookkeeping and anger decay/Expert phase are not covered by the empty-world native oracle.");
        true
    }

    /// Other simulation subsystems report campaign gaps through the same
    /// deterministic queue, independently of combat's presentation events.
    pub(crate) fn report_mission_diagnostic(
        &mut self,
        trigger: Option<&str>,
        kind: DiagnosticKind,
        message: &str,
    ) {
        if let Some(state) = self.mission_state.as_mut() {
            state.diagnose(self.tick, trigger, kind, message);
        }
    }

    pub fn unit_mission(&self, id: u32) -> Option<UnitMission> {
        self.mission_state
            .as_ref()?
            .unit_missions
            .iter()
            .find(|state| state.unit == id)
            .map(|state| state.mission)
    }

    pub(crate) fn clear_unit_mission(&mut self, id: u32) {
        if let Some(state) = self.mission_state.as_mut() {
            state.unit_missions.retain(|state| state.unit != id);
        }
    }

    /// Assign a supported original Mission without pretending it is a team
    /// script opcode. Team membership remains until original team logic frees it.
    pub fn assign_unit_mission(&mut self, id: u32, mission: UnitMission) -> bool {
        if self.mission_state.is_none() || self.unit(id).is_none() {
            return false;
        }
        self.order(crate::Order::Stop { unit: id });
        let now = self.tick;
        let special = self.unit(id).is_some_and(|unit| {
            mission == UnitMission::Hunt
                && (unit.stats.deploys_into.is_some() || unit.stats.fighter.engineer)
        });
        if let Some(unit) = self.units.iter_mut().find(|unit| unit.id == id) {
            if mission == UnitMission::AreaGuard {
                unit.post = Some(unit.cell());
            }
        }
        let state = self.mission_state.as_mut().unwrap();
        state.unit_missions.push(UnitMissionState {
            unit: id,
            mission,
            assigned_at: now,
            next_scan: now + 1,
        });
        if special {
            state.diagnose(now, None, DiagnosticKind::UnitMissionSpecialCase { unit: id, mission },
                "This engineer or deployable vehicle has an original special Hunt branch that is not implemented; its Hunt execution remains stopped.");
        }
        if matches!(
            mission,
            UnitMission::Hunt | UnitMission::AreaGuard | UnitMission::Harmless
        ) {
            state.diagnose(now, None, DiagnosticKind::UnitMissionTargetingUnverified { mission },
                "The mission runs with the simulator's current target selection; original target priority, Harmless NoThreat handling, and within-tick execution order still need differential verification.");
        }
        true
    }

    /// Infantry.SpawnParachuted queues Guard/Hunt directly. Routing that
    /// through an ordinary Stop order would defer Stop until landing and then
    /// erase this queued mission again.
    pub(crate) fn queue_parachuted_mission(&mut self, id: u32, human: bool) {
        let mission = if human { UnitMission::Guard } else { UnitMission::Hunt };
        if let Some(state) = self.mission_state.as_mut() {
            state.unit_missions.retain(|entry| entry.unit != id);
            state.unit_missions.push(UnitMissionState { unit: id, mission, assigned_at: self.tick, next_scan: self.tick + 1 });
            if !human {
                state.diagnose(self.tick, None, DiagnosticKind::UnitMissionTargetingUnverified { mission },
                    "The original parachuted AI Infantry Hunt mission is queued; its eventual target choice uses the simulator's existing targeting model.");
            }
        }
    }

    pub(crate) fn unit_mission_recruitable(&self, id: u32) -> bool {
        !matches!(
            self.unit_mission(id),
            Some(UnitMission::Sleep | UnitMission::Hunt | UnitMission::Harmless)
        )
    }

    pub(crate) fn unit_mission_retaliates(&self, id: u32) -> bool {
        !matches!(
            self.unit_mission(id),
            Some(UnitMission::Sleep | UnitMission::Hunt | UnitMission::Harmless)
        )
    }

    pub(crate) fn unit_mission_scatters(&self, id: u32) -> bool {
        self.unit_mission(id) != Some(UnitMission::Sleep)
    }

    pub(crate) fn unit_mission_passive(&self, id: u32) -> bool {
        !matches!(
            self.unit_mission(id),
            Some(UnitMission::Sleep | UnitMission::Hunt | UnitMission::Harmless)
        )
    }

    pub(crate) fn run_unit_missions(&mut self) {
        let Some(state) = self.mission_state.as_ref() else {
            return;
        };
        let hunters: Vec<_> = state
            .unit_missions
            .iter()
            .filter(|state| state.mission == UnitMission::Hunt && self.tick >= state.next_scan)
            .map(|state| state.unit)
            .collect();
        for id in hunters {
            let Some(unit) = self.unit(id) else {
                continue;
            };
            if unit.health <= 0
                || unit.falling.is_some()
                || unit.tethered
                || self.tick < unit.warp_until
                || unit.stats.fighter.engineer
                || unit.stats.deploys_into.is_some()
            {
                continue;
            }
            let target = unit
                .arms
                .target
                .filter(|&target| self.alive(target))
                .or_else(|| self.team_threat(id, 0, false));
            if let Some(unit) = self.units.iter_mut().find(|unit| unit.id == id) {
                unit.arms.target = target;
                unit.arms.attacking = target.is_some();
                unit.arms.passive = false;
            }
            let rate = self
                .mission_data
                .as_ref()
                .and_then(|data| {
                    data.unit_mission_rates
                        .iter()
                        .find(|(mission, _)| *mission == UnitMission::Hunt)
                })
                .map_or(14, |(_, rate)| *rate);
            let next = self.tick + rate as u64 + (self.next_random() % 3) as u64;
            if let Some(state) = self.mission_state.as_mut().and_then(|state| {
                state
                    .unit_missions
                    .iter_mut()
                    .find(|state| state.unit == id)
            }) {
                state.next_scan = next;
            }
        }
    }

    /// Called only after restore has read the mission state. A save cannot be
    /// silently loaded onto different or absent mission rules.
    pub(crate) fn validate_mission_state(&self) -> Result<(), String> {
        match (&self.mission_data, &self.mission_state) {
            (None, None) => Ok(()),
            (Some(data), Some(state))
                if data.id == state.id
                    && data.rules_hash() == state.rules_hash
                    && self.mission_team_rules_hash() == state.team_rules_hash =>
            {
                if state.tags.len() < data.tags.len() {
                    return Err("mission tag state omits map TagType slots".into());
                }
                for (tag_index, tag) in state.tags.iter().enumerate() {
                    if data.tags.get(tag.definition).is_none_or(|definition| tag.id != definition.id)
                        || (tag_index < data.tags.len() && (tag.definition != tag_index || tag.team.is_some()))
                        || (!tag.instantiated && !tag.triggers.is_empty())
                        || tag.triggers.iter().any(|&index| {
                            state
                                .triggers
                                .get(index)
                                .is_none_or(|trigger| trigger.tag != tag_index)
                        })
                    {
                        return Err("mission tag state does not match its rules".into());
                    }
                }
                for trigger in &state.triggers {
                    if trigger.tag >= state.tags.len()
                        || data.trigger(&trigger.id).is_none_or(|definition| {
                            definition.events.len() != trigger.occurred.len()
                        })
                    {
                        return Err("mission trigger state does not match its rules".into());
                    }
                }
                if state.event_houses.len() != data.triggers.len() || state.event_houses.iter().zip(&data.triggers).any(|((id, events), definition)| id != &definition.id || events.len() != definition.events.len()) {
                    return Err("mission shared event state does not match its rules".into());
                }
                if state.diplomacy.len() != data.houses.len() || state.diplomacy.iter().zip(&data.houses).any(|(entry, binding)|
                    entry.house != binding.house || entry.anger.iter().any(|node| !data.houses.iter().any(|binding| binding.house == node.house))) {
                    return Err("mission diplomacy state does not match its House registry".into());
                }
                for team in &self.teams {
                    if team.scenario_tag.is_some_and(|tag| state.tags.get(tag).is_none_or(|tag| tag.team != Some(team.id))) {
                        return Err("mission Team Tag does not match its constructor instance".into());
                    }
                }
                Ok(())
            }
            _ => {
                Err("snapshot mission rules are absent or differ from this world's mission".into())
            }
        }
    }

    /// No mission means no state change and no random-number consumption.
    pub(crate) fn run_mission(&mut self) {
        let Some(data) = self.mission_data.take() else {
            return;
        };
        let Some(mut state) = self.mission_state.take() else {
            self.mission_data = Some(data);
            return;
        };
        if state.outcome.is_some() {
            self.mission_data = Some(data);
            self.mission_state = Some(state);
            return;
        }
        // Preserve visible gaps for registered House/map notification paths,
        // without inventing a global timer source for those registrations.
        for tag in state.tags.iter().filter(|tag| tag.instantiated && tag.team.is_none() && !tag.periodic && !tag.destroyed && !tag.pending_delete).cloned().collect::<Vec<_>>() {
            for index in tag.triggers {
                if !state.triggers[index].enabled || state.triggers[index].destroyed { continue; }
                let definition = data.trigger(&state.triggers[index].id).expect("validated map trigger");
                for (index, event) in definition.events.iter().enumerate() {
                    if event.kind != 8 && event_attach_flags(event.kind, data.yuri) & 0x04 != 0 {
                        state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: event.kind, index },
                            "This active map notification source is not implemented; it has not been replaced by a global timer poll.");
                    }
                }
            }
        }
        // Logic visits its registered Tags in order. Each Tag fully dispatches
        // RegisterEvent -> FireActions -> next Trigger, then immediately sends
        // Notice before the next Tag. Enable never calls its target recursively;
        // a target later in these existing collections can still run this pass.
        let polling: Vec<usize> = state.tags.iter().enumerate().filter(|(_, tag)| tag.instantiated && tag.periodic && !tag.destroyed && !tag.pending_delete)
            .map(|(index, _)| index).collect();
        for tag in polling {
            self.raise_tag_mission_event(&data, &mut state, tag, MissionEventContext { source: None, kind: 13, object: None, cell: None });
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    fn related_house_mission_tags(data: &MissionData, state: &MissionState, house: u8) -> Vec<usize> {
        // Scenario registers one GetInstance per authored Map TagType with
        // flags8, to its FirstTrigger's static Country. New Team Tag instances
        // and other linked Trigger countries do not acquire this registration.
        data.tags.iter().enumerate().filter_map(|(index, tag)| {
            let instance = &state.tags[index];
            if !instance.instantiated || instance.destroyed || instance.pending_delete || instance.team.is_some() { return None; }
            let owner = data.trigger(&tag.trigger).and_then(|trigger| data.country_house(&trigger.country));
            (owner == Some(house) && data.tag_attach_flags(tag) & 8 != 0).then_some(index)
        }).collect()
    }

    pub(crate) fn run_house_mission_tags(&mut self, house: u8) {
        if self.tick % 8 != 0 || self.mission_simulation_stopped() { return; }
        let Some(data) = self.mission_data.take() else { return; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return; };
        let count = Self::related_house_mission_tags(&data, &state, house).len();
        let complete = {
            let mut context = (&mut *self, &data, &mut state);
            visit_house_tags_live(context.0.tick, &mut context, count,
                |(_, data, state), index| Self::related_house_mission_tags(data, state, house).get(index).copied(),
                |(world, data, state), tag| world.raise_tag_mission_event(data, state, tag, MissionEventContext { source: None, kind: 8, object: None, cell: None }))
        };
        if !complete {
            state.diagnose(self.tick, None, DiagnosticKind::UnsupportedEvent { kind: 8, index: house as usize },
                "A House callback removed enough RelatedTags for the original loop to index past its new Count. Native stale backing-vector pointers are not modeled; this source pass stops at that bounded lifecycle gap.");
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    /// Mirrors the registered global Techno.Array identities, including Foot
    /// objects retained in cargo, garrisons, bio-reactors and parasite leaps.
    /// No owner, health, on-map or DontScore filter exists in native YR60/61.
    fn registered_techno_count(&self, wanted: MissionTechnoKind) -> (i32, usize) {
        fn foot(unit: &crate::Unit, wanted: MissionTechnoKind, seen: &mut std::collections::BTreeSet<u32>, count: &mut i32) {
            if !seen.insert(unit.id) { return; }
            if let MissionTechnoKind::Unit(index) = wanted {
                if unit.stats.kind.checked_sub(1) == Some(index) { *count += 1; }
            }
            for child in unit.cargo.iter().chain(&unit.parasite) { foot(child, wanted, seen, count); }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut count = 0;
        for unit in &self.units { foot(unit, wanted, &mut seen, &mut count); }
        for building in &self.buildings {
            if seen.insert(building.id) && wanted == MissionTechnoKind::Building(building.kind) { count += 1; }
            for unit in building.occupants.iter().chain(building.patients.iter().map(|(_, unit)| unit)) { foot(unit, wanted, &mut seen, &mut count); }
        }
        for leap in &self.leaps { foot(&leap.biter, wanted, &mut seen, &mut count); }
        (count, seen.len())
    }

    /// Original Factory.Set creates and globally registers its current product
    /// before Unlimbo. The simulator keeps that object as Production parameters
    /// until placement/release; each occupied current slot supplies one such
    /// identity. Queued future types do not construct objects and do not count.
    fn factory_techno_count(&self, wanted: MissionTechnoKind) -> (i32, usize) {
        let mut count = 0;
        let mut registered = 0;
        for house in &self.houses { for tab in 0..6 {
            if let Some(production) = house.tab_slot(tab) {
                registered += 1;
                if match wanted {
                    MissionTechnoKind::Building(index) => tab < 2 && production.building == index,
                    MissionTechnoKind::Unit(index) => tab >= 2 && production.building == index,
                } { count += 1; }
            }
        }}
        (count, registered)
    }

    fn mission_conditions(&self, data: &MissionData, state: &mut MissionState, index: usize, context: MissionEventContext) -> bool {
        let definition = data.trigger(&state.triggers[index].id).expect("validated mission trigger");
        let persistence = data.tags[state.tags[state.triggers[index].tag].definition].persistence;
        let mut all = true;
        // Native Register passes the same writable repeat argument through
        // the whole event chain. A successful helper can change it for later
        // events; an already remembered event skips its helper entirely.
        let mut repeat = persistence == 2;
        // RegisterEvent resolves Type.House.ArrayIndex through CountryFind
        // for HasOccured. YR's separate runtime Trigger.House is an output
        // of accepted event.GetHouse calls, not this input argument.
        let predicate_house = data.country_house(&definition.country);
        for (event_index, event) in definition.events.iter().enumerate() {
            let occurred = if state.triggers[index].occurred[event_index] { true } else { match event.kind {
                0 => false,
                1 | 59 if event.kind == 1 || data.yuri => {
                    // FindByCountryIndex returns the first actual House of the
                    // Country, rather than accepting every House of that type.
                    // Scenario bindings exclude World's synthetic neutral slot.
                    let owner = context.object.and_then(|object| self.owner_of(object));
                    let required = data.houses.iter().filter(|binding| self.houses.get(binding.house as usize).is_some_and(|house| house.country as i32 == event.parameter)).map(|binding| binding.house).next();
                    let matches = context.kind == event.kind && owner.is_some() && (event.parameter == -1 || owner == required);
                    if matches { repeat = true; }
                    if matches && data.yuri {
                        state.event_houses.iter_mut().find(|(id, _)| id == &definition.id).expect("validated shared events").1[event_index] = owner;
                    }
                    matches
                }
                6 | 7 | 48 => context.kind == event.kind,
                8 => true,
                23 => {
                    if context.kind != 23 { false }
                    else if data.country_house(&definition.country).is_none() { true }
                    else {
                        let kind = data.team_types.iter().find(|(id, _)| id.eq_ignore_ascii_case(&event.argument)).map(|(_, kind)| *kind);
                        if kind.is_none() {
                            state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::MissingTeamType { team_type: event.argument.clone() },
                                "LeavesMap references an unresolved original TeamType ID; a different team has not been substituted.");
                        }
                        let matches = self.teams.iter().any(|team| Some(team.kind) == kind && team.members.is_empty() && team.leaving_map);
                        if matches { repeat = true; }
                        matches
                    }
                }
                32 | 57 => {
                    // Value is a BuildingType index. The House argument comes
                    // from the static TriggerType Country on every Register,
                    // independently of YR EnteredBy's runtime House output.
                    if let Some(house) = predicate_house {
                        if event.parameter < 0 {
                            state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::InvalidParameter { kind: event.kind, index: event_index },
                                "The original Counter getter indexes negative BuildingType values before its array. That unsafe native path is bounded as unsatisfied.");
                            false
                        } else {
                            // Counter.GetItemCount expands nonnegative slots
                            // past its capacity with zero. A missing registry
                            // slot therefore stays zero without inventing a type.
                            let kind = event.parameter as usize;
                            let exists = kind < self.building_types.len()
                                && !(data.yuri && data.building_dont_score.contains(&(kind as u16)))
                                && self.buildings.iter().any(|building| building.owner == house && building.kind as usize == kind);
                            let matches = exists == (event.kind == 32);
                            if matches { repeat = true; }
                            matches
                        }
                    } else {
                        // Native null-House falls through its later Country
                        // branch and returns true for both32/57, leaving the
                        // shared repeat argument untouched. Original CPU checks
                        // cover this surprising behavior directly.
                        true
                    }
                }
                44 => {
                    // Value is the attacking Techno's original House.ArrayIndex,
                    // not its Country or the victim/passed Trigger House.
                    let owner = context.source.and_then(|source| match source {
                        crate::combat::Aim::Unit(id) => self.registered_unit(id).map(|unit| unit.owner),
                        crate::combat::Aim::Building(id) => self.buildings.iter().find(|building| building.id == id).map(|building| building.owner),
                        crate::combat::Aim::Cell(..) => None,
                    });
                    if context.kind != 44 || context.source.is_none() { false }
                    else if let Some(owner) = owner {
                        if let Some(original_index) = data.houses.iter().position(|binding| binding.house == owner) {
                            event.parameter == original_index as i32
                        } else {
                            state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: 44, index: event_index },
                                "The source Techno's House has no original House.ArrayIndex binding. Its runtime slot/Country is not substituted for event44's original index; dynamic or synthetic House creation remains unverified.");
                            false
                        }
                    } else {
                        state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: 44, index: event_index },
                            "The attacking Techno's deferred lifetime/owner is not represented. Event44 does not infer a source House from its victim or a previous shot.");
                        false
                    }
                }
                58 if data.yuri => {
                    // This parameter is a Country index. It deliberately does
                    // not use the current runtime Trigger.House argument.
                    let house = usize::try_from(event.parameter).ok().and_then(|index| data.country_names.get(index)).and_then(|country| data.country_house(country));
                    if let Some(house) = house {
                        state.diagnose(self.tick, None, DiagnosticKind::CampaignAiUnverified { house, feature: "MissionPowerAccounting".into() },
                            "PowerFull uses the original Country selection and PowerPercent predicate, with this engine's current power totals. Complete original RecheckPower dirty-cache accounting, construction/selling/ownership/blackout update phases are not yet differentially verified.");
                        let (output, drain) = self.power(house);
                        native_power_full(output, drain)
                    } else {
                        // Native FindByCountryIndex returning null is success,
                        // including -1 and an unbound or unknown registry index.
                        true
                    }
                }
                60 | 61 if data.yuri => {
                    let name = event.argument.as_bytes();
                    if !name.is_ascii() {
                        state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: event.kind, index: event_index },
                            "A non-ASCII TechnoType ID needs the original INI code-page binding; no case-folded or Unicode replacement is accepted.");
                        false
                    } else {
                        // TEvent.ReadINI copies at most24 source bytes and adds
                        // a terminator before HasOccurred's exact byte compare.
                        let name = &name[..name.len().min(24)];
                        let mut bindings = data.techno_types.iter().filter(|binding| binding.id.as_bytes() == name);
                        let found = bindings.next();
                        if bindings.next().is_some() {
                            state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: event.kind, index: event_index },
                                "The original reverse TechnoType.Array scan resolves duplicate exact IDs by registration order. The shared registry's cross-class order is not bound yet, so this ambiguous predicate remains unsatisfied.");
                            false
                        } else if let Some(binding) = found {
                            let (count, registered) = self.registered_techno_count(binding.kind);
                            let (products, factories) = self.factory_techno_count(binding.kind);
                            if event.kind == 61 { count + products == 0 }
                            else { registered + factories > 0 && count + products >= event.parameter }
                        } else {
                            if data.techno_types.is_empty() {
                                state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: event.kind, index: event_index },
                                    "The campaign has no original TechnoType registry binding. A missing binding cannot prove that an authored type is absent.");
                            }
                            // Both native60/61 return false for an unknown ID;
                            // 61 does not interpret an unknown name as absence.
                            false
                        }
                    }
                }
                13 if event.parameter >= 0 => self.tick.saturating_sub(state.triggers[index].timer.start_tick) >= state.triggers[index].timer.length_ticks,
                47 if event.parameter >= 0 => self.tick / TICKS_PER_SECOND as u64 >= event.parameter as u64,
                27 if (0..data.global_variable_limit).contains(&event.parameter) => state.global(event.parameter),
                28 if (0..data.global_variable_limit).contains(&event.parameter) => !state.global(event.parameter),
                36 if (0..data.local_variable_limit).contains(&event.parameter) => state.local(event.parameter),
                37 if (0..data.local_variable_limit).contains(&event.parameter) => !state.local(event.parameter),
                13 | 27 | 28 | 36 | 37 | 47 => {
                    state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::InvalidParameter { kind: event.kind, index: event_index },
                        "A timer parameter is negative or a variable ID lies outside the original fixed array.");
                    false
                }
                _ => {
                    state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: event.kind, index: event_index },
                        "An active mission event is not implemented; its condition remains unsatisfied.");
                    false
                }
            }};
            if occurred && data.yuri {
                if let Some(house) = state.event_houses.iter().find(|(id, _)| id == &definition.id).and_then(|(_, events)| events[event_index]) {
                    state.triggers[index].house = Some(house);
                }
            }
            // In RegisterEvent only repeating/persistent events accepted by
            // StateA and StateB enter the remembered bitfield. Of the supported
            // kinds, 7 and 48 retain the persistence2 input repeat value.
            // Event23 with a nonnull House explicitly sets repeat=true, even
            // for persistence0/1, so a partial AND can outlive the empty Team.
            if repeat && matches!(event.kind, 7 | 23 | 48) && occurred { state.triggers[index].occurred[event_index] = true; }
            all &= occurred;
        }
        all
    }

    pub(crate) fn has_registered_mission_object(&self, object: crate::combat::Aim) -> bool {
        match object {
            crate::combat::Aim::Unit(id) => self.registered_unit(id).is_some(),
            crate::combat::Aim::Building(id) => self.buildings.iter().any(|building| building.id == id),
            crate::combat::Aim::Cell(..) => false,
        }
    }

    fn expire_mission_tag(&mut self, state: &mut MissionState, tag: usize) {
        state.tags[tag].pending_delete = true;
        state.tags[tag].periodic = false;
        // Native AnnounceExpiredPointer synchronously clears attached tags on
        // registered objects, including off-map ones, before queueing the Tag
        // destructor. Already
        // deleted identities do not silently decrement a leaked native count.
        state.tags[tag].objects.retain(|&object| !self.has_registered_mission_object(object));
        state.tags[tag].cells.clear();
        // Team participates in the same Notice vector. Its PointerExpired
        // clears the raw Team.Tag pointer, without changing InstanceCount.
        for team in &mut self.teams {
            if team.scenario_tag == Some(tag) { team.scenario_tag = None; }
        }
    }

    pub(crate) fn finish_mission_deletions(&mut self) {
        if let Some(state) = self.mission_state.as_mut() {
            let expired: Vec<_> = state.tags.iter().enumerate().filter(|(_, tag)| tag.pending_delete && !tag.destroyed).map(|(index, _)| index).collect();
            for tag in expired { state.destroy_tag(tag); }
        }
    }

    fn raise_tag_mission_event(&mut self, data: &MissionData, state: &mut MissionState, tag_index: usize, context: MissionEventContext) {
        if state.tags.get(tag_index).is_none_or(|tag| tag.pending_delete) { return; }
        self.raise_cached_tag_mission_event(data, state, tag_index, context);
    }

    // A native cached Tag pointer can survive Notice clearing all live
    // attachments and queuing deletion. RaiseEvent guards Destroyed and
    // IsExecuting, not queue membership. Ordinary lookups still reject pending.
    fn raise_cached_tag_mission_event(&mut self, data: &MissionData, state: &mut MissionState, tag_index: usize, context: MissionEventContext) {
        if state.tags.get(tag_index).is_none_or(|tag| !tag.instantiated || tag.destroyed || tag.executing) { return; }
        let tag = &data.tags[state.tags[tag_index].definition];
        if tag.persistence > 2 {
            state.diagnose(self.tick, None, DiagnosticKind::UnsupportedPersistence { persistence: tag.persistence }, "This attached Tag has an unknown persistence value.");
            return;
        }
        let mut detach = false;
        let mut expire = false;
        state.tags[tag_index].executing = true;
        for index in state.tags[tag_index].triggers.clone() {
            if !state.triggers[index].enabled || state.triggers[index].destroyed { continue; }
            if !self.mission_conditions(data, state, index, context) { continue; }
            if tag.persistence == 0 { detach = true; }
            if tag.persistence == 1 && state.tag_refs(tag_index) != 1 { detach = true; continue; }
            let definition = data.trigger(&state.triggers[index].id).expect("validated attached trigger");
            if tag.persistence == 2 { state.reset(index, definition, self.tick); }
            state.triggers[index].fire_count += 1;
            state.outputs.push(MissionOutput::TriggerFired { tick: self.tick, trigger_id: definition.id.clone(), tag_id: tag.id.clone() });
            for (action_index, action) in definition.actions.iter().enumerate() { self.mission_action(data, state, definition, index, action_index, action); }
            if tag.persistence < 2 { state.triggers[index].destroyed = true; expire = true; }
        }
        // Original +35 clears before the separate attachment/Notice phase.
        state.tags[tag_index].executing = false;
        if let Some(object) = context.object.filter(|&object| detach && state.object_tag(object) == Some(tag_index)) { state.attach_object_tag(object, None); }
        if let Some(cell) = context.cell.filter(|_| detach) { state.tags[tag_index].cells.retain(|&other| other != cell); }
        if expire { self.expire_mission_tag(state, tag_index); }
    }

    fn raise_object_mission_event(&mut self, data: &MissionData, state: &mut MissionState, object: crate::combat::Aim, kind: i32) {
        if let Some(tag) = state.object_tag(object) { self.raise_tag_mission_event(data, state, tag, MissionEventContext { source: None, kind, object: Some(object), cell: None }); }
    }

    /// Techno.SetOwningHouse invokes RegisterDestruction(null) before changing
    /// Owner. This is a live-object notification, not a final death: recording
    /// it in `destructions` would wrongly suppress a later attacker's Event7.
    fn raise_owner_change_events(&mut self, data: &MissionData, state: &mut MissionState, object: crate::combat::Aim) {
        self.raise_object_mission_event(data, state, object, 48);
        let alive = match object {
            crate::combat::Aim::Unit(id) => self.registered_unit(id).is_some_and(|unit| unit.health > 0),
            crate::combat::Aim::Building(id) => self.buildings.iter().any(|building| building.id == id && building.health > 0),
            crate::combat::Aim::Cell(..) => false,
        };
        if alive { self.raise_object_mission_event(data, state, object, 29); }
    }

    /// Team.Update walks the registered Logic Tags forwards; UpdateStrength,
    /// called synchronously by LiberateMember, walks them backwards. Neither
    /// broadcasts to independently constructed object/Team Tag instances.
    pub(crate) fn register_mission_team_exit(&mut self, reverse: bool) {
        let Some(data) = self.mission_data.take() else { return; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return; };
        self.raise_mission_team_exit_with(&data, &mut state, reverse);
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    fn raise_mission_team_exit_with(&mut self, data: &MissionData, state: &mut MissionState, reverse: bool) {
        let mut tags: Vec<_> = state.tags.iter().enumerate().filter(|(_, tag)| tag.instantiated && tag.periodic && !tag.pending_delete && !tag.destroyed).map(|(index, _)| index).collect();
        if reverse { tags.reverse(); }
        for tag in tags {
            self.raise_tag_mission_event(data, state, tag, MissionEventContext { source: None, kind: 23, object: None, cell: None });
        }
    }

    /// Foot::UpdatePosition(PCP.End) raises1 and, in YR,59 on the same cached
    /// Cell.Tag. Each gets fresh map coordinates. Placement is no source.
    pub(crate) fn register_mission_cell_entry(&mut self, unit: u32) {
        if self.complete_campaign_building_entry(unit) { return; }
        let Some(foot) = self.unit(unit).filter(|unit| unit.health > 0) else { return; };
        let cell = foot.cell();
        // Accepted entry branches return before their Foot callback. Their
        // destination Tag uses a separate source with EmptyCell location.
        let entry = foot.capture.and_then(|building| self.buildings.iter().find(|other| other.id == building))
            .filter(|building| (building.cell.0..building.cell.0 + building.size.0).contains(&cell.0) && (building.cell.1..building.cell.1 + building.size.1).contains(&cell.1))
            .and_then(|building| self.entry_action(unit, building.id));
        let yuri = self.mission_data.as_ref().is_some_and(|data| data.yuri);
        if entry.is_some() && (foot.stats.infantry || (yuri && matches!(entry, Some(crate::combat::Entry::Grind | crate::combat::Entry::Absorb)))) { return; }
        // Native flags 0x500 reject the ground object beneath a high bridge.
        // Map represents its high-bridge heads/spans through bit0; remaining
        // native bridge-construction variants retain a geometry diagnostic.
        if self.map.bridged(cell.0, cell.1) && !foot.on_bridge { return; }
        // RA2 skips the CellTag source for fully cloaked Foot; YR removed
        // this early exit. This is independent of the event's Country test.
        if foot.cloak.state == 2 && self.mission_data.as_ref().is_some_and(|data| !data.yuri) { return; }
        let Some(data) = self.mission_data.take() else { return; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return; };
        if state.outcome.is_none() {
            if let Some(tag) = state.cell_tag(cell) {
                self.raise_tag_mission_event(&data, &mut state, tag, MissionEventContext { source: None, kind: 1, object: Some(crate::combat::Aim::Unit(unit)), cell: (cell != (0, 0)).then_some(cell) });
                if data.yuri {
                    if let Some(cell) = self.unit(unit).map(|foot| foot.cell()) {
                        if state.tags[tag].pending_delete {
                            state.diagnose(self.tick, None, DiagnosticKind::CellEntryUnverified { feature: "CachedTagDeletionQueue".into() },
                                "PCP.End retains the original CellTag pointer for59 after1 queues its deletion. Its remaining live Triggers can still register; duplicate native delete-queue entries and subsequent double-destruction behavior are not represented by the simulator's single pending slot.");
                        }
                        self.raise_cached_tag_mission_event(&data, &mut state, tag, MissionEventContext { source: None, kind: 59, object: Some(crate::combat::Aim::Unit(unit)), cell: (cell != (0, 0)).then_some(cell) });
                    } else {
                        state.diagnose(self.tick, None, DiagnosticKind::CellEntryUnverified { feature: "CachedFootLifetime".into() },
                            "EnteredBy removed or moved its Foot out of the placed root collection before the cached59 call. Native deferred object lifetime and its fresh GetMapCoords/owner are not reconstructed from stale placement data.");
                    }
                }
            }
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    /// YR Foot.Update's airborne CellTag source. Root membership represents
    /// ordinary successful Mark(Down) placement; it is not a geometry test for
    /// native IsOnMap. Unbound map/limbo states are explicit unsupported bounds.
    pub(crate) fn register_mission_airborne_event(&mut self, unit: u32) -> bool {
        if self.tick % 16 != 0 || !self.mission_data.as_ref().is_some_and(|data| data.yuri) { return false; }
        let Some(foot) = self.unit(unit) else { return false; }; // Unlimbo-free cargo is not a placed root Foot.
        let cell = foot.cell();
        let unsupported = if foot.chrono.is_some() || self.tick < foot.warp_until { Some(("AirborneOnMapState", "Chronosphere/warp placement and Mark(Up/Down) transitions are not bound to native IsOnMap for this source.")) }
            else if foot.rocket.is_some() || foot.stats.rocket || foot.stats.missile_spawn { Some(("AirborneSpecialCoordinates", "Rocket/missile Foot placement and coordinate getters need their original source mapping.")) }
            else if self.map.index(cell.0, cell.1).is_none() { Some(("AirborneMapBoundary", "A placed root Foot outside the represented map is not assumed to have native IsOnMap=false. Its native GetCell/floor mapping is unverified, so this source is bounded without raising.")) }
            else if self.map.ramp(cell.0, cell.1) != 0 { Some(("AirborneFloorHeight", "The original GetHeight subtracts Map.GetFloorHeight, including sloped terrain. This source currently supports flat cells only.")) }
            else if foot.on_bridge && !self.map.bridged(cell.0, cell.1) { Some(("AirborneBridgeLayer", "This Foot's OnBridge placement does not resolve to a represented bridge deck; its original relative floor is unverified.")) }
            else { None };
        // On flat mapped cells the public coordinates provide absoluteZ;
        // GetHeight subtracts the terrain floor and, on a deck,416 leptons.
        let height = unsupported.is_none().then(|| {
            let absolute_z = self.aim_point(crate::combat::Aim::Unit(unit)).expect("placed Foot").2;
            absolute_z.wrapping_sub(self.ground_z(foot.x, foot.y)).wrapping_sub(if foot.on_bridge { 4 * crate::combat::LEVEL_HEIGHT } else { 0 })
        });
        let Some(data) = self.mission_data.take() else { return false; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return false; };
        let mut raised = false;
        if state.outcome.is_none() {
            if let Some((feature, message)) = unsupported {
                if state.tags.iter().any(|tag| tag.instantiated && !tag.destroyed && !tag.pending_delete && !tag.cells.is_empty()) {
                    state.diagnose(self.tick, None, DiagnosticKind::CellEntryUnverified { feature: feature.into() }, message);
                }
            } else if height.is_some_and(|height| height >= 2 * crate::combat::LEVEL_HEIGHT) && state.cell_tag(cell).is_some() {
                // Native first GetCell only tests Tag presence, then calls
                // GetMapCoords, then GetCell again to read the current Tag.
                let location = self.unit(unit).expect("no action between getters").cell();
                let current_cell = self.unit(unit).expect("no action between getters").cell();
                if let Some(tag) = state.cell_tag(current_cell) {
                    self.raise_tag_mission_event(&data, &mut state, tag, MissionEventContext { source: None, kind: 59, object: Some(crate::combat::Aim::Unit(unit)), cell: (location != (0, 0)).then_some(location) });
                    raised = true;
                }
            }
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
        raised
    }

    /// Accepted Infantry entry targets the Building's own Tag, but the
    /// entering Infantry remains pTagOwner and location is EmptyCell.
    pub(crate) fn register_mission_building_entry(&mut self, unit: u32, building: u32) {
        let Some(data) = self.mission_data.take() else { return; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return; };
        if state.outcome.is_none() {
            if let Some(tag) = state.object_tag(crate::combat::Aim::Building(building)) {
                self.raise_tag_mission_event(&data, &mut state, tag, MissionEventContext { source: None, kind: 1, object: Some(crate::combat::Aim::Unit(unit)), cell: None });
            }
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    /// Known ordinary destruction or consumed-entry source, before removal.
    /// A nonnull attacker permits 7; 48 is also raised with a null attacker.
    /// Erase/consume/conversion paths must choose their own native source.
    pub(crate) fn register_mission_destruction(&mut self, object: crate::combat::Aim, has_attacker: bool) {
        let Some(data) = self.mission_data.take() else { return; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return; };
        if state.outcome.is_none() && state.object_tag(object).is_some() && !state.destructions.contains(&object) {
            state.destructions.push(object);
            if has_attacker { self.raise_object_mission_event(&data, &mut state, object, 7); }
            self.raise_object_mission_event(&data, &mut state, object, 48);
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    /// Positive, ordinary, nonlethal Object.ReceiveDamage notification tail.
    /// Existing damage/death callers decide whether this accepted tail occurs;
    /// lethal7/48 and specialized MindControl sources are separate contracts.
    pub(crate) fn register_mission_attacked(&mut self, victim: crate::combat::Aim, source: Option<crate::combat::Aim>) {
        let Some(source) = source.filter(|source| !matches!(source, crate::combat::Aim::Cell(..))) else { return; };
        let Some(data) = self.mission_data.take() else { return; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return; };
        if state.outcome.is_none() {
            if self.has_registered_mission_object(source) {
                let mut context = (&mut *self, &data, &mut state);
                visit_ordinary_damage_notifications(&mut context, 1, true,
                    |(world, _, _)| world.has_registered_mission_object(victim),
                    |(_, _, state)| state.object_tag(victim),
                    |(world, data, state), tag, kind| world.raise_tag_mission_event(data, state, tag,
                        MissionEventContext { kind, object: Some(victim), source: Some(source), cell: None }));
            } else if let Some(tag) = state.object_tag(victim) {
                for index in state.tags[tag].triggers.clone() {
                    let definition = data.trigger(&state.triggers[index].id).expect("validated attached Trigger");
                    for (index, event) in definition.events.iter().enumerate().filter(|(_, event)| matches!(event.kind, 6 | 44)) {
                        state.diagnose(self.tick, Some(&definition.id), DiagnosticKind::UnsupportedEvent { kind: event.kind, index },
                            "The ordinary damage source has left the registered Techno collection before its notification tail. Native deferred source lifetime is unverified; a stale identity is not treated as an owning House.");
                    }
                }
            }
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    pub(crate) fn diagnose_mission_mind_control_notification(&mut self, target: crate::combat::Aim) {
        let Some(state) = self.mission_state.as_mut() else { return; };
        if state.object_tag(target).is_some() {
            state.diagnose(self.tick, None, DiagnosticKind::ChangeHouseUnverified { object: Some(target), feature: "MindControlDamageNotification".into() },
                "The native MindControl bullet sends6/44 before attempting CaptureManager.Capture. Only ordinary ReceiveDamage notifications are integrated; this specialized pre-capture source remains unverified and is not synthesized from an ownership change.");
        }
    }

    pub(crate) fn transfer_mission_tag(&mut self, from: crate::combat::Aim, to: crate::combat::Aim) {
        if let Some(state) = self.mission_state.as_mut() {
            if let Some(tag) = state.object_tag(from) {
                state.attach_object_tag(to, Some(tag));
                state.attach_object_tag(from, None);
            }
        }
    }

    pub(crate) fn share_capture_mission_tag(&mut self, engineer: u32, building: u32) {
        let (Some(data), Some(state)) = (&self.mission_data, self.mission_state.as_mut()) else { return; };
        let Some(tag) = state.object_tag(crate::combat::Aim::Unit(engineer)) else { return; };
        if state.tags[tag].triggers.iter().any(|&index| data.trigger(&state.triggers[index].id).is_some_and(|definition| definition.must_transfer)) {
            // AttachTrigger replaces the building's former attachment, but
            // retains the engineer's: count increases before the engineer's48.
            state.attach_object_tag(crate::combat::Aim::Building(building), Some(tag));
        }
    }

    fn create_team_tag_with(&mut self, data: &MissionData, state: &mut MissionState, team_id: u32) -> Result<(), String> {
        let Some(team) = self.teams.iter().find(|team| team.id == team_id) else { return Err("mission team disappeared before Tag construction".into()); };
        if team.scenario_tag.is_some() { return Ok(()); }
        let Some(tag) = data.team_definitions.iter().find(|definition| definition.kind == team.kind).and_then(|definition| definition.tag.as_ref()) else { return Ok(()); };
        let Some(definition) = data.tags.iter().position(|definition| definition.id.eq_ignore_ascii_case(tag)) else { return Err(format!("TeamType.Tag {tag:?} has no TagType definition")); };
        let index = state.new_team_tag(data, definition, team_id, self.tick)?;
        self.teams.iter_mut().find(|team| team.id == team_id).expect("mission team").scenario_tag = Some(index);
        Ok(())
    }

    pub(crate) fn initialize_campaign_team_tag(&mut self, team: u32) {
        let Some(data) = self.mission_data.take() else { return; };
        let Some(mut state) = self.mission_state.take() else { self.mission_data = Some(data); return; };
        if let Err(message) = self.create_team_tag_with(&data, &mut state, team) {
            let tag = self.teams.iter().find(|other| other.id == team).and_then(|team| data.team_definitions.iter().find(|definition| definition.kind == team.kind)).and_then(|definition| definition.tag.clone()).unwrap_or_default();
            state.diagnose(self.tick, None, DiagnosticKind::MissingTagType { tag }, &message);
        }
        self.mission_data = Some(data);
        self.mission_state = Some(state);
    }

    fn attach_team_member_tag_with(&self, data: &MissionData, state: &mut MissionState, team_id: u32, unit: u32, unit_kind: u16) {
        let Some(team) = self.teams.iter().find(|team| team.id == team_id) else { return; };
        let Some(tag) = team.scenario_tag else { return; };
        let only_transport = data.team_definitions.iter().find(|definition| definition.kind == team.kind)
            .and_then(|definition| definition.properties.iter().rev().find(|(key, _)| key.eq_ignore_ascii_case("OnTransOnly")))
            .is_some_and(|(_, value)| matches!(value.trim().to_ascii_lowercase().as_str(), "yes" | "true" | "1"));
        let carrier = (unit_kind as usize).checked_sub(1).and_then(|index| self.unit_types.get(index)).is_some_and(|kind| kind.passengers > 0);
        if !only_transport || carrier { state.attach_object_tag(crate::combat::Aim::Unit(unit), Some(tag)); }
    }

    pub(crate) fn attach_campaign_team_member_tag(&mut self, team: u32, unit: u32) {
        let Some(data) = self.mission_data.take() else { return; };
        if let Some(mut state) = self.mission_state.take() {
            if let Some(unit_kind) = self.unit(unit).map(|unit| unit.stats.kind) { self.attach_team_member_tag_with(&data, &mut state, team, unit, unit_kind); }
            self.mission_state = Some(state);
        }
        self.mission_data = Some(data);
    }

    pub(crate) fn liberate_campaign_tag(&mut self, team: u32, unit: u32, owner: u8) {
        if self.houses.get(owner as usize).is_none_or(|house| house.human) { return; }
        let tag = self.teams.iter().find(|other| other.id == team).and_then(|team| team.scenario_tag);
        if let (Some(tag), Some(state)) = (tag, self.mission_state.as_mut()) {
            let object = crate::combat::Aim::Unit(unit);
            if state.object_tag(object) == Some(tag) { state.attach_object_tag(object, None); }
        }
    }

    pub(crate) fn destroy_empty_team_tag(&mut self, tag: Option<usize>) {
        if let (Some(tag), Some(state)) = (tag, self.mission_state.as_mut()) {
            if state.tag_refs(tag) == 0 { state.destroy_tag(tag); }
        }
    }

    fn reinforce_ordinary(
        &mut self,
        data: &MissionData,
        state: &mut MissionState,
        original: &str,
        waypoint_override: Option<i32>,
    ) -> Result<(), (DiagnosticKind, String)> {
        let missing = || (DiagnosticKind::MissingTeamType { team_type: original.into() }, "Reinforcement references a TeamType without its simulation binding.".into());
        let Some(kind) = data.team_types.iter().find(|(id, _)| id.eq_ignore_ascii_case(original)).map(|(_, kind)| *kind) else { return Err(missing()); };
        let Some(definition) = data.team_definitions.iter().find(|definition| definition.kind == kind) else { return Err(missing()); };
        let gap = |feature: &str, message: &str| (DiagnosticKind::ReinforcementUnverified { team_type: original.into(), feature: feature.into() }, message.to_owned());
        let property = |key: &str| definition.properties.iter().rev().find(|(name, _)| name.eq_ignore_ascii_case(key)).map(|(_, value)| value.as_str());
        let yes = |key: &str| property(key).is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "yes" | "true" | "1"));
        let droppod = yes("Droppod");
        if let Some(tag) = &definition.tag {
            if !data.tags.iter().any(|definition| definition.id.eq_ignore_ascii_case(tag)) { return Err((DiagnosticKind::MissingTagType { tag: tag.clone() }, "Reinforcement's TeamType.Tag has no TagType definition.".into())); }
        }
        let Some(house) = data.country_house(&definition.house_country) else {
            return Err((DiagnosticKind::MissingCountryHouse { country: definition.house_country.clone() }, "Reinforcement uses TeamType.House, which has no runtime House binding.".into()));
        };
        let Some(waypoint) = waypoint_override.or(definition.waypoint) else {
            return Err((DiagnosticKind::MissingWaypoint { waypoint: "-1".into() }, "Reinforcement has no effective TeamType/action waypoint.".into()));
        };
        let Some(&(_, seed)) = data.waypoints.iter().find(|(id, _)| *id == waypoint) else {
            return Err((DiagnosticKind::MissingWaypoint { waypoint: waypoint.to_string() }, "Reinforcement's effective waypoint is absent.".into()));
        };
        let Some(team_type) = self.team_data.team_types.get(kind).cloned() else { return Err(missing()); };
        let Some(force) = self.team_data.task_forces.get(team_type.task_force).cloned() else { return Err(gap("TaskForce", "Reinforcement has no valid TaskForce definition.")); };
        if force.is_empty() || !force.iter().any(|&(count, _)| count > 0) { return Err(gap("EmptyTaskForce", "No reinforcement members can be constructed. The original unsafe empty-head path is rejected explicitly.")); }
        let Some(script) = self.team_data.scripts.get(team_type.script) else { return Err(gap("ScriptReference", "Reinforcement references a missing ScriptType; no replacement definition was invented.")); };
        let empty_script = script.is_empty();
        for &(count, unit_kind) in &force {
            if count == 0 { continue; }
            let Some(unit_type) = self.unit_types.get(unit_kind as usize) else { return Err(gap("UnitTypeReference", "Reinforcement has an unresolved TaskForce member type.")); };
            let stats = unit_type.stats;
            if droppod && !stats.infantry {
                return Err(gap("DroppodCargoClass", "This delivery currently supports nonnested Infantry cargo. Vehicle, aircraft and nested transport parachuting need their original class-specific SpawnParachuted paths."));
            }
            if stats.jet.is_some() || stats.plane.is_some() || stats.rocket || stats.missile_spawn {
                return Err(gap("AirLocomotor", "Aircraft, Jumpjet and rocket reinforcement altitude/entry missions are not implemented by ordinary ground placement."));
            }
            if stats.movement_zone == 6 || data.unit_locomotors.iter().any(|(index, clsid)| *index == unit_kind && clsid.to_ascii_uppercase().contains("4A582743")) {
                return Err(gap("TunnelLocomotor", "All-Tunnel reinforcement uses its own underground Unlimbo and locomotor Move_To sequence."));
            }
        }
        let full = yes("Full");
        let delivery = if droppod {
            let Some(plane_kind) = self.super_rules.plane.filter(|&kind| self.unit_types.get(kind as usize).is_some_and(|kind| kind.stats.plane.is_some())) else {
                return Err(gap("PDPLANE", "The original literal PDPLANE aircraft type is not bound; no substitute plane or ground spawn was created."));
            };
            let explicit_origin = yes("UseTransportOrigin");
            let origin = if explicit_origin {
                let Some(origin) = definition.transport_waypoint.and_then(|waypoint| data.waypoints.iter().find(|(id, _)| *id == waypoint).map(|(_, cell)| *cell)).filter(|cell| *cell != (0, 0)) else {
                    return Err(gap("DroppodTransportWaypoint", "UseTransportOrigin requires its original nonempty TransportWaypoint. No random edge fallback was invented."));
                };
                origin
            } else {
                let edge = data.house_edges.iter().find(|entry| entry.0 == house).map_or(-1, |entry| entry.1);
                let Some(origin) = self.campaign_paradrop_edge_for(edge, data.yuri, u32::MAX) else {
                    return Err(gap("DroppodRandomEdge", "The Winged origin needs known Size/LocalSize, flat edge candidates and a valid YR Owner.Edge (or remaining mutable StartingEdge state). No generic top-edge replacement was created."));
                };
                origin
            };
            if (explicit_origin && !self.map.in_playfield(origin.0, origin.1)) || self.map.index(origin.0, origin.1).is_none() || self.map.bridged(origin.0, origin.1) || self.map.ramp(origin.0, origin.1) != 0 {
                return Err(gap("DroppodOriginUnlimbo", "This plane origin needs the remaining hidden-border, bridge or slope Unlimbo path."));
            }
            Some((plane_kind, origin))
        } else { None };
        // These scans use TaskForce type entries even when their Amount is
        // zero. In YR a naval member excludes nonnaval Unit carriers; RA2 has
        // no such filter. Aircraft carriers remain a separate placement path.
        let has_air_carrier = force.iter().any(|&(_, index)| self.unit_types.get(index as usize).is_some_and(|kind| kind.passengers > 0 && kind.stats.plane.is_some()));
        let has_naval = force.iter().any(|&(_, index)| self.unit_types.get(index as usize).is_some_and(|kind| kind.naval));
        let carrier_kinds: Vec<u16> = if full && !has_air_carrier {
            force.iter().filter_map(|&(_, index)| self.unit_types.get(index as usize)
                .filter(|kind| kind.passengers > 0 && !kind.stats.infantry && kind.stats.plane.is_none() && (!data.yuri || !has_naval || kind.naval))
                .map(|_| index)).collect()
        } else { Vec::new() };
        if seed == (0, 0) { return Err(gap("EmptyPlacementCell", "The original empty-cell sentinel cannot place a reinforcement chain.")); }
        let only_infantry = force.iter().filter(|&&(count, _)| count > 0).all(|&(_, index)| self.unit_types[index as usize].stats.infantry);
        if !droppod && only_infantry && (-1..=1).any(|y| (-1..=1).any(|x| self.building_on((seed.0 + x, seed.1 + y)).is_some())) {
            return Err(gap("InfantryBuildingInsertion", "The all-infantry chain has a building at or around its waypoint. The original special-building insertion predicate and exit sequence are not yet implemented."));
        }
        if !self.map.in_playfield(seed.0, seed.1) {
            return Err(gap("HiddenBorderEntry", "The original waypoint is outside the usable area. Full-Size border cell storage and its entry movement are not reconstructed; the waypoint was not moved to a nearby playable cell."));
        }
        if !droppod && self.map.bridged(seed.0, seed.1) {
            return Err(gap("BridgeUnlimbo", "Original bridge placement height and bridge-content layer still require the original Unlimbo path."));
        }
        let head_kind = force.iter().rev().find(|&&(count, kind)| count > 0 && carrier_kinds.contains(&kind))
            .or_else(|| force.iter().rev().find(|&&(count, _)| count > 0)).expect("positive member").1;
        let has_cargo = force.iter().any(|&(count, kind)| count > 0 && carrier_kinds.contains(&kind)) && force.iter().any(|&(count, kind)| count > 0 && !carrier_kinds.contains(&kind));
        if data.yuri && has_cargo && data.unit_open_topped.contains(&head_kind) {
            return Err(gap("OpenToppedPassengerBehavior", "This Full head carrier is OpenTopped. Passenger location propagation and combat must be implemented before it can be created as a faithful loaded transport."));
        }
        let explicit_inside = waypoint_override.is_some();
        let mut cell = seed;
        if explicit_inside && !droppod {
            // Native NearByLocation starts with radius zero, retains duplicate
            // seed candidates, then uses frame % candidates. Verify the subset
            // whose ground visibility and occupancy match our existing helper.
            let mut probe = crate::Unit::new(u32::MAX - 1, house, seed, 0, self.unit_types[head_kind as usize].stats);
            probe.stats.movement_zone = 0;
            let level = self.map.level(seed.0, seed.1);
            let flat = |at: (i32, i32)| self.map.index(at.0, at.1).is_some_and(|index| !self.map.overlay[index])
                && !self.map.bridged(at.0, at.1) && self.map.ramp(at.0, at.1) == 0
                && self.map.level(at.0, at.1) == level;
            let visible_flat = |at: (i32, i32)| (0..=6).all(|step| flat((at.0 + step, at.1 + step)));
            let free = |at: (i32, i32)| self.map.index(at.0, at.1).is_some_and(|index| self.claims[index] == 0 && self.spots[index] & crate::ALL_SPOT_BITS == 0
                && self.map.obstacle[index] == crate::Obstacle::None as u8 && !self.map.overlay[index])
                && self.map.in_playfield(at.0, at.1) && self.map.speed_on(at.0, at.1, probe.stats.speed_type, true) > 0;
            let clean = free(seed) && visible_flat(seed);
            let surrounded = !free(seed) && crate::DIRECTIONS.iter().all(|&(x, y)| {
                let at = (seed.0 + x, seed.1 + y);
                free(at) && visible_flat(at)
            });
            if !clean && !surrounded {
                return Err(gap("NearByLocation", "Action 80's original NearByLocation is verified here only for a clear flat seed or a blocked seed with eight clear flat neighbors. This terrain/occupancy needs the remaining original search predicates."));
            }
            let Some(found) = crate::nearby(&self.map, &self.claims, &self.spots, &probe, seed, crate::Near::new(true, false, None), self.tick) else {
                return Err(gap("NearByLocation", "No valid original nearby candidate was available in the implemented subset."));
            };
            cell = found;
        }
        let facing = if explicit_inside { 0 } else {
            data.house_edges.iter().find(|entry| entry.0 == house).map_or(0, |(_, edge)| if *edge < 0 { 0 } else { edge.wrapping_mul(64) as u8 })
        };
        let team = self.create_reinforcement_team(house, kind, original, seed)
            .map_err(|message| (DiagnosticKind::TeamCreationFailed { team_type: original.into() }, message))?;
        self.create_team_tag_with(data, state, team).map_err(|message| (DiagnosticKind::TeamCreationFailed { team_type: original.into() }, message))?;
        if empty_script && !state.default_script_overrides.contains(&team_type.script) { state.default_script_overrides.push(team_type.script); }
        let veteran = property("VeteranLevel").and_then(|value| value.trim().parse::<i32>().ok()).unwrap_or(1);
        let mut ordinary = Vec::new();
        let mut carriers = Vec::new();
        // CreateObject/AddMember are in numeric TaskForce row order. Detach and
        // Unlimbo through Object.Next in reverse successful creation order.
        for (count, unit_kind) in force {
            for _ in 0..count {
                let stats = crate::UnitStats { kind: unit_kind + 1, ..self.unit_types[unit_kind as usize].stats };
                // CreateObject has no placement arguments: off-map passengers
                // retain the ctor's zero Location and north Facing. Only the
                // returned chain's later Unlimbo assigns seed and Edge.
                let mut unit = self.make_unit((0, 0), 0, stats, house);
                unit.x = 0;
                unit.y = 0;
                if matches!(veteran, 0 | 2 | 3) { unit.veterancy = if veteran == 0 { 0.0 } else { f64::from(veteran - 1) }; unit.shown_rank = unit.rank(); }
                if full && stats.storage > 0 { unit.work.cargo[0] = stats.storage; }
                self.initialize_reinforcement_member(team, &mut unit);
                self.attach_team_member_tag_with(data, state, team, unit.id, unit.stats.kind);
                state.unit_groups.push((unit.id, definition.group));
                if carrier_kinds.contains(&unit_kind) { carriers.push(unit); }
                else { ordinary.push(unit); }
            }
        }
        let members = if let Some(head) = carriers.last_mut() {
            // Passengers.AddPassenger accepts the entire ordinary Object.Next
            // chain once. Vec keeps the reverse of that linked order because
            // ordinary unloading pops the native head first. Capacity/size
            // checks from player boarding do not belong to this constructor.
            head.cargo.append(&mut ordinary);
            carriers
        } else { ordinary };
        if let Some((plane_kind, origin)) = delivery {
            let aircraft = self.create_campaign_paradrop(plane_kind, house, origin, seed, members);
            state.diagnose(self.tick, None, DiagnosticKind::ParadropUnverified { aircraft, feature: "FlightAndFallingModel".into() },
                "The original independent PDPLANE, constructor passenger chain, approach/overfly queue cadence, retry-head behavior and original Team retention run. Flight uses the existing stats-aware Fly model; complete native flight, falling, parachute animation and transport-loss callbacks still require differential verification.");
            if !yes("UseTransportOrigin") {
                state.diagnose(self.tick, None, DiagnosticKind::ParadropUnverified { aircraft, feature: "NativeRandomStream".into() },
                    "The Winged edge geometry and inclusive random request order follow the original flat subset. The simulator's current RNG differs from the original 250-word Scenario stream, so equal original seeds are not claimed.");
            }
        } else {
          let index = self.map.index(cell.0, cell.1).expect("supported reinforcement cell");
          for mut unit in members.into_iter().rev() {
            let id = unit.id;
            (unit.x, unit.y) = crate::cell_centre(cell);
            unit.set_facing(facing);
            if unit.stats.infantry {
                // Original Infantry.Unlimbo passes ignoreContents=true to
                // FindSubposition during ScenarioInit: every centre input
                // stays in slot 0, without a three-soldier cap or RNG draw.
                self.spots[index] |= 1;
                self.spot_house[index] = house;
            } else { self.claim_scenario_cell(index, id); }
            state.unit_missions.push(UnitMissionState { unit: id, mission: UnitMission::Guard, assigned_at: self.tick, next_scan: self.tick });
            self.put_back(unit);
          }
        }
        state.diagnose(self.tick, None, DiagnosticKind::ReinforcementUnverified { team_type: original.into(), feature: "ObjectInitializationAndFrameOrder".into() },
            if droppod { "Original Team/member construction and the independent Droppod aircraft are connected. Full original constructor side effects and within-frame traversal still need full-game comparison." }
            else { "Ordinary ground/naval construction, owner, numeric row order, reverse placement, same-cell list, facing, veterancy, Guard and deferred script entry are implemented. Original constructor side effects and within-frame traversal still need full-game comparison." });
        if has_cargo {
            state.diagnose(self.tick, None, DiagnosticKind::ReinforcementUnverified { team_type: original.into(), feature: "TransportUnloadingAndLoss".into() },
                "Full carrier/cargo chains, Team membership, shared Tags and ordinary unload ordering run. Complete original transport exit/radio timing and per-passenger destruction/escape callbacks when the carrier dies are not yet verified.");
        }
        if self.allied(house, data.player_house) {
            state.diagnose(self.tick, None, DiagnosticKind::ReinforcementUnverified { team_type: original.into(), feature: "AlliedFlare".into() }, "The original allied-owner reinforcement flare presentation is not emitted yet.");
        }
        Ok(())
    }

    pub(crate) fn diagnose_campaign_paradrop(&mut self, aircraft: u32, feature: &str, message: &str) {
        if let Some(state) = self.mission_state.as_mut() {
            state.diagnose(self.tick, None, DiagnosticKind::ParadropUnverified { aircraft, feature: feature.into() }, message);
        }
    }

    fn change_attached_building_house(&mut self, data: &MissionData, state: &mut MissionState, trigger: &TriggerDef, id: u32, owner: u8) {
        use crate::combat::Aim;
        let object = Aim::Building(id);
        let Some(building) = self.buildings.iter().find(|building| building.id == id) else { return; };
        let Some(kind) = self.building_types.get(building.kind as usize) else { return; };
        let from = building.owner;
        if from == owner { return; }
        let tabs: &[usize] = match kind.factory {
            Some(crate::Factory::Building) => &[0, 1],
            Some(crate::Factory::Infantry) => &[2],
            Some(crate::Factory::Unit) if kind.naval => &[4],
            Some(crate::Factory::Unit) => &[3],
            Some(crate::Factory::Aircraft) => &[5],
            None => &[],
        };
        let production = self.houses.get(from as usize).is_some_and(|house| tabs.iter().any(|&tab| house.tab_slot(tab).is_some() || (tab >= 2 && !house.queue(tab).is_empty())));
        // Scenario Structure constructors zero their four resource floats;
        // the INI loader never fills them. Both editions' ordinary refinery
        // dumps and YR slave returns credit House cash directly. A deployed
        // or otherwise converted object's resource transfer is a separate
        // source, so its absence from the original Structure bindings matters.
        let initial_storage = data.objects.iter().any(|binding| binding.object == object && binding.section.eq_ignore_ascii_case("Structures"));
        let unsupported = if building.health <= 0 || self.dying.iter().any(|(who, _)| *who == object) { Some("DestructionWindow") }
            else if building.buildup.is_some() { Some("ConstructionOrSellingOwnership") }
            else if production { Some("ActiveBuildingFactoryOwnership") }
            else if building.unload.is_some() || building.docked.is_some() || building.refining.is_some() || building.depot_busy || building.docks.iter().any(Option::is_some) { Some("BuildingRadioOrDockOwnership") }
            else if !building.occupants.is_empty() || !building.patients.is_empty() { Some("OccupiedBuildingOwnership") }
            else if building.frozen.is_some() || building.c4.is_some() { Some("TemporalOrC4BuildingOwnership") }
            else if kind.plug || kind.slave_spawner.is_some() || building.charge.is_some() || building.supports != 0 || building.supporting.is_some() || !building.chargers.is_empty() { Some("SpecialBuildingOwnership") }
            else if kind.storage > 0 && !initial_storage { Some("StoredResourceBuildingOwnership") }
            else { None };
        if let Some(feature) = unsupported {
            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::ChangeHouseUnverified { object: Some(object), feature: feature.into() },
                "This Building needs an additional original ownership branch. Active production, resource contents, radio contacts and attached objects are preserved by leaving its current owner unchanged.");
            return;
        }
        let factory = kind.factory;
        let naval = kind.naval;
        let (startup, _, delay) = kind.cash;
        let primary = factory.is_some() && !self.buildings.iter().any(|other| other.id != id && other.owner == owner && other.primary && self.building_types.get(other.kind as usize).is_some_and(|kind| kind.factory == factory && kind.naval == naval));
        let building = self.buildings.iter_mut().find(|building| building.id == id).unwrap();
        // Building's prefix precedes the Techno old-owner callback. Preserve
        // ammunition, firing cooldown, facing and health throughout transfer.
        building.primary = false;
        building.captured = true;
        building.arms.target = None;
        building.arms.attacking = false;
        if factory.is_some() || kind.construction_yard { building.rally = None; }
        self.raise_owner_change_events(data, state, object);
        let Some(building) = self.buildings.iter_mut().find(|building| building.id == id) else { return; };
        building.owner = owner;
        building.primary = primary;
        building.repairing = false;
        building.looked = false;
        if delay != 0 { building.cash_at = self.tick.saturating_add(delay.max(0) as u64); }
        if startup != 0 && data.passive_houses.contains(&from) {
            self.houses[owner as usize].credits += startup;
        }
        state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::ChangeHouseUnverified { object: Some(object), feature: "BuildingOwnerTransitionSideEffects".into() },
            "ChangeHouse transfers an idle Building with native captured/primary/repair/rally and cash-timer behavior. Power, build access and SpySat follow its new owner through existing queries; exact native dirty-flag scheduling and the complete radio/special-building branches remain unverified.");
    }

    fn change_attached_house(&mut self, data: &MissionData, state: &mut MissionState, trigger: &TriggerDef, runtime: usize, index: usize, parameter: &str) {
        use crate::combat::Aim;
        let Ok(country) = parameter.trim().parse::<i32>() else {
            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::InvalidParameter { kind: 14, index }, "ChangeHouse needs an original Country index.");
            return;
        };
        if country == -1 {
            if !data.yuri {
                state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::ChangeHouseUnverified { object: None, feature: "InvalidCountryNullOwnership".into() },
                    "RA2's original inline action can pass a null House for an unresolved Country. That unsafe source path is explicitly bounded rather than copied as a null-owner change.");
            }
            return;
        }
        let owner = if data.yuri && country == 8997 { state.triggers[runtime].house }
            else { usize::try_from(country).ok().and_then(|index| data.country_names.get(index)).and_then(|name| data.country_house(name)) };
        let Some(owner) = owner else {
            if !(data.yuri && country == 8997) {
                state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingCountryHouse { country: country.to_string() },
                    "ChangeHouse could not resolve its original Country index to the first matching runtime House; multiplayer-address values require their own Scenario binding.");
            }
            return;
        };
        let native_human = data.human_player_houses.as_ref().map_or_else(|| self.houses[owner as usize].human, |houses| houses.contains(&owner));
        // Native Tag.ContainsTrigger compares the actual Trigger instance.
        // Team Tags with the same authored ID must not capture one another.
        let mut objects: Vec<Aim> = state.tags.iter().filter(|tag| tag.instantiated && !tag.pending_delete && !tag.destroyed && tag.triggers.contains(&runtime))
            .flat_map(|tag| tag.objects.iter().copied()).collect();
        objects.sort_by_key(|object| match object { Aim::Unit(id) | Aim::Building(id) => *id, Aim::Cell(..) => u32::MAX });
        objects.dedup();
        for object in objects {
            if self.owner_of(object) == Some(owner) { continue; }
            let Aim::Unit(id) = object else {
                if let Aim::Building(id) = object { self.change_attached_building_house(data, state, trigger, id, owner); }
                continue;
            };
            let Some(unit) = self.unit(id) else { continue; }; // Off-map cargo is excluded by native IsOnMap/IsInLimbo.
            let unsupported = if unit.health <= 0 || self.dying.iter().any(|(who, _)| *who == object) { Some("DestructionWindow") }
                else if unit.plane.is_some() || unit.jet.is_some() || unit.falling.is_some() || unit.on_bridge { Some("AirOrBridgeOwnership") }
                else if !unit.cargo.is_empty() || !unit.parasite.is_empty() || unit.spawner.is_some() { Some("NestedOrSpawnerOwnership") }
                else if unit.controlled_by.is_some() || unit.permanently_controlled || unit.controls.is_some() { Some("MindControlOwnership") }
                else if unit.frozen.is_some() || unit.chrono.is_some() || self.tick < unit.warp_until { Some("WarpOwnership") }
                else if unit.deploying || matches!(unit.stance, crate::combat::Stance::Deploying(_) | crate::combat::Stance::Undeploying(_)) { Some("ConversionOwnership") }
                else if unit.stats.harvester && unit.work.job != crate::Job::Idle { Some("HarvestMissionOwnership") }
                else if unit.tethered || unit.work.dock.is_some() || unit.unloading.is_some() { Some("DockOrUnloadOwnership") }
                else { None };
            if let Some(feature) = unsupported {
                state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::ChangeHouseUnverified { object: Some(object), feature: feature.into() },
                    "This attached object needs an additional original SetOwningHouse branch. It remains under its current owner instead of losing that state through a generic capture.");
                continue;
            }
            let unit = self.units.iter_mut().find(|unit| unit.id == id).unwrap();
            unit.arms.target = None;
            unit.arms.attacking = false;
            unit.capture = None;
            unit.boarding = None;
            unit.enter_area = None;
            crate::give_up(unit);
            if data.yuri { unit.work.archive = None; }
            // The old Owner remains visible to the registration callback.
            // In the ordinary Action14 source its own Tag is executing and
            // therefore rejects this nested RaiseEvent, as in both originals.
            self.raise_owner_change_events(data, state, object);
            let Some(unit) = self.units.iter_mut().find(|unit| unit.id == id) else { continue; };
            unit.owner = owner;
            unit.looked = None;
            if unit.stats.infantry {
                for (x, y) in [(unit.x, unit.y), unit.head_to.unwrap_or((unit.x, unit.y))] {
                    if let Some(at) = self.map.index(x.div_euclid(crate::LEPTONS_PER_CELL), y.div_euclid(crate::LEPTONS_PER_CELL)) { self.spot_house[at] = owner; }
                }
            }
            if data.yuri {
                state.unit_missions.retain(|entry| entry.unit != id);
                state.unit_missions.push(UnitMissionState { unit: id, mission: UnitMission::Guard, assigned_at: self.tick, next_scan: self.tick + 1 });
            }
            if data.yuri && native_human {
                if let Some((team, tag, leaving)) = self.release_member_to_human_owner(id) {
                    if leaving { self.raise_mission_team_exit_with(data, state, true); }
                    self.delete_team(team);
                    if let Some(tag) = tag.filter(|&tag| state.tag_refs(tag) == 0) { state.destroy_tag(tag); }
                }
            }
            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::ChangeHouseUnverified { object: Some(object), feature: "OwnerTransitionSideEffects".into() },
                "ChangeHouse transfers ordinary on-map Foot objects by the current runtime Trigger and clears their target/destination. YR also clears Archive, queues Guard and liberates a new native-human owner's Team membership while retaining its Tag; RA2 preserves its prior mission and Team. Complete original radio, motion, sensors and within-frame owner-change side effects still need differential verification.");
        }
    }

    /// Retail action63 calls DamageArea five times, not ReceiveDamage once.
    /// Its null source still permits destruction48 and any actions nested in
    /// that notification. Publish the current dispatch state while combat runs
    /// so those callbacks see the live executing Tag and return their changes.
    fn damage_mission_waypoint(&mut self, data: &MissionData, state: &mut MissionState,
        trigger: &TriggerDef, index: usize, parameter: &str) {
        let waypoint = crate::diplomacy::atoi32(parameter);
        let Some(cell) = data.waypoints.iter().find(|(id, _)| *id == waypoint)
            .filter(|_| (0..702).contains(&waypoint)).map(|(_, cell)| *cell)
            .filter(|cell| self.map.index(cell.0, cell.1).is_some()) else {
            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingWaypoint { waypoint: parameter.into() },
                "Action63 calls the original waypoint-coordinate getter before its later validity check. An undefined or unrepresented cell cannot be reproduced through a safe invented blast position; this path remains bounded here.");
            return;
        };
        let Some(warhead) = self.ability_rules.c4_warhead.filter(|&id| self.warheads.get(id as usize).is_some()) else {
            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::InvalidParameter { kind: 63, index },
                "Action63 requires the General.C4Warhead rules binding; no replacement warhead or direct health subtraction was used.");
            return;
        };
        debug_assert!(self.mission_data.is_none() && self.mission_state.is_none());
        self.mission_data = Some(data.clone());
        self.mission_state = Some(state.clone());
        for (dx, dy) in [(0, 0), (85, 85), (-85, 85), (85, -85), (-85, -85)] {
            // GetWaypointCoords is called again before every DamageArea, so a
            // bridge removed by an earlier blast changes the next call's Z.
            let (x, y) = crate::cell_centre(cell);
            let z = self.ground_z(x, y) + if self.map.bridged(cell.0, cell.1) { 4 * crate::combat::LEVEL_HEIGHT } else { 0 };
            self.explode((x + dx, y + dy, z), 100, warhead, None);
        }
        // IsWaypointDefined is checked only after the five damage calls.
        // EmptyCell is not defined, despite GetWaypointCoords accepting it.
        if cell != (0, 0) {
            for _ in 0..4 {
                if self.damage_mission_bridge(cell) { break; }
            }
        }
        *state = self.mission_state.take().expect("combat retains mission state");
        self.mission_data = None;
    }

    fn mission_action(
        &mut self,
        data: &MissionData,
        state: &mut MissionState,
        trigger: &TriggerDef,
        runtime_trigger: usize,
        index: usize,
        action: &ActionDef,
    ) {
        let parameter = action.parameters[1].as_str();
        match action.kind {
            63 => self.damage_mission_waypoint(data, state, trigger, index, parameter),
            14 => self.change_attached_house(data, state, trigger, runtime_trigger, index, parameter),
            7 | 80 => {
                let override_waypoint = if action.kind == 80 {
                    let Some(waypoint) = decode_waypoint(&action.parameters[6]) else {
                        state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingWaypoint { waypoint: action.parameters[6].clone() }, "Action 80 requires a valid encoded waypoint; TeamType.Waypoint is not its fallback.");
                        return;
                    };
                    Some(waypoint)
                } else { None };
                if let Err((kind, message)) = self.reinforce_ordinary(data, state, parameter, override_waypoint) {
                    state.diagnose(self.tick, Some(&trigger.id), kind, &message);
                }
            }
            3 | 13 | 74 | 75 => {
                // TAction.Country is a Country registry index. -1 is the
                // original no-house sentinel, not the trigger's owning house.
                if parameter.trim() == "-1" { return; }
                let country = parameter.parse::<usize>().ok().and_then(|index| data.country_names.get(index));
                let house = country.and_then(|country| data.country_house(country));
                let Some(house) = house else {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingCountryHouse { country: country.cloned().unwrap_or_else(|| parameter.into()) },
                        "The AI switch could not resolve its Country registry index to a runtime House.");
                    return;
                };
                if self.houses[house as usize].human { return; }
                let brain = self.houses[house as usize].ai.get_or_insert_with(|| crate::ai::Brain::for_campaign(2 - data.difficulty.min(2) as usize, crate::ai::CampaignState::default()));
                let control = brain.campaign.get_or_insert_with(crate::ai::CampaignState::default);
                match action.kind {
                    3 => { control.production = true; brain.awake = true; }
                    13 => control.autocreate = true,
                    74 => brain.triggers_active = true,
                    75 => brain.triggers_active = false,
                    _ => unreachable!(),
                }
            }
            30 | 76 => {
                let Some(house) = data.country_house(&trigger.country) else {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingCountryHouse { country: trigger.country.clone() },
                        "The contextual AI switch could not resolve the trigger's owning Country.");
                    return;
                };
                if let Some(brain) = self.houses[house as usize].ai.as_mut() {
                    if action.kind == 76 {
                        if let Ok(ratio) = parameter.parse::<i32>() { brain.trigger_ratio = ratio; }
                        else { state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::InvalidParameter { kind: action.kind, index }, "RatioAITriggerTeam requires an integer percentage."); }
                        return;
                    }
                    if let Some(control) = brain.campaign.as_mut() {
                        control.auto_base_building = parameter.parse::<i32>().is_ok_and(|value| value != 0);
                        if control.auto_base_building {
                            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::CampaignAiUnverified { house, feature: "AutoBaseBuilding".into() },
                                "Authored Base nodes and team production run; automatic campaign base-plan extension has not yet been reconstructed.");
                        }
                    }
                }
            }
            0 => {},
            4 => {
                let kind = data.team_types.iter().find(|(id, _)| id.eq_ignore_ascii_case(parameter)).map(|(_, kind)| *kind);
                let definition = kind.and_then(|kind| data.team_definitions.iter().find(|definition| definition.kind == kind));
                let (Some(kind), Some(definition)) = (kind, definition) else {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingTeamType { team_type: parameter.into() },
                        "Create Team references a TeamType without its map-to-simulation binding.");
                    return;
                };
                let Some(house) = data.country_house(&definition.house_country) else {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingCountryHouse { country: definition.house_country.clone() },
                        "Create Team uses TeamType.House, which could not be resolved to a runtime House.");
                    return;
                };
                let waypoint = match definition.waypoint {
                    Some(id) => match data.waypoints.iter().find(|(other, _)| *other == id) {
                        Some((_, cell)) => Some(*cell),
                        None => {
                            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingWaypoint { waypoint: id.to_string() },
                                "Create Team's TeamType waypoint does not exist; no team was created at an invented location.");
                            return;
                        }
                    },
                    None => None,
                };
                match self.create_team(house, kind) {
                    Ok(id) => {
                        if let Err(message) = self.create_team_tag_with(data, state, id) {
                            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingTagType { tag: definition.tag.clone().unwrap_or_default() }, &message);
                        }
                        let team = self.teams.iter_mut().find(|team| team.id == id).expect("just-created mission team");
                        team.mission_type = Some(parameter.into());
                        team.scenario_waypoint = waypoint;
                        state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::TeamRecruitmentUnverified { team_type: parameter.into() },
                            "The original action-4 empty-team construction, owner, Max bypass and waypoint/group preference are implemented. Full recruitment eligibility and within-tick ordering still require original dynamic comparison.");
                        if definition.loose_recruit {
                            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::UnsupportedTeamProperty { team_type: parameter.into(), property: "LooseRecruit".into() },
                                "LooseRecruit is retained but its original recruitment eligibility has not yet been verified.");
                        }
                    }
                    Err(message) => state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::TeamCreationFailed { team_type: parameter.into() }, &message),
                }
            }
            11 => state.outputs.push(MissionOutput::Text { label: parameter.into() }),
            37 | 38 => { self.mission_diplomacy_action(data, state, trigger, runtime_trigger, index, action); },
            19 => state.outputs.push(MissionOutput::Sound { name: parameter.into() }),
            21 => state.outputs.push(MissionOutput::Speech { name: parameter.into() }),
            46 | 47 => state.input_locked = action.kind == 46,
            53 | 54 => {
                let mut found = false;
                for target in 0..state.triggers.len() {
                    // The original action scans Trigger.Array until deferred
                    // destruction removes its entries. A Trigger.Destroyed
                    // byte alone does not exclude it from Enable/Disable.
                    if !state.triggers[target].id.eq_ignore_ascii_case(parameter) || state.tags[state.triggers[target].tag].destroyed { continue; }
                    found = true;
                    let definition = data.trigger(&state.triggers[target].id).expect("validated mission target");
                    if action.kind == 54 {
                        state.triggers[target].enabled = false;
                    } else if data.difficulty_allows(definition) {
                        state.triggers[target].enabled = true;
                        // Every Enable resets, including Enable of an enabled
                        // target. It never recursively dispatches the target.
                        state.reset(target, definition, self.tick);
                    }
                }
                if !found {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingRuntimeTrigger { target: parameter.into() },
                        "Enable/disable did not find a live instance of the target trigger.");
                }
            }
            28 | 29 | 56 | 57 => {
                let Ok(id) = parameter.parse::<i32>() else {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::InvalidParameter { kind: action.kind, index }, "A mission variable ID is not an integer.");
                    return;
                };
                let global = action.kind == 28 || action.kind == 29;
                let limit = if global { data.global_variable_limit } else { data.local_variable_limit };
                if !(0..limit).contains(&id) {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::InvalidParameter { kind: action.kind, index }, "A mission variable ID lies outside the original fixed array; the original ignores this assignment.");
                    return;
                }
                assign_mission_variable(data, state, self.tick, global, id, matches!(action.kind, 28 | 56));
            }
            1 | 2 => {
                let Ok(country) = parameter.trim().parse::<i32>() else {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::InvalidParameter { kind: action.kind, index },
                        "Win/lose requires its original signed Country parameter.");
                    return;
                };
                let house = data.player_house;
                // Both original wrappers compare the raw parameter only to
                // CurrentPlayer.Type.ArrayIndex. No Country/House lookup is
                // performed, including for unknown and negative parameters.
                let victory = (action.kind == 1) == (country == self.houses[house as usize].country as i32);
                let accepted = state.endgame.request(victory, state.outcome.is_some() || self.over.is_some(), self.tick, data.savour_delay_ticks);
                let human = self.houses[house as usize].human || data.human_player_houses.as_ref().is_some_and(|houses| houses.contains(&house));
                if accepted && human {
                    // Original campaign House.Win/Lose announce immediately,
                    // before their SavourDelay and bounded audio expiry wait.
                    // The helper calls the text renderer before EVA.Speak.
                    state.outputs.push(MissionOutput::Text { label: if victory { "TXT_SCENARIO_WON" } else { "TXT_SCENARIO_LOST" }.into() });
                    state.outputs.push(MissionOutput::Speech { name: if victory { "EVA_MissionAccomplished" } else { "EVA_MissionFailed" }.into() });
                }
            }
            17 | 18 | 48 | 55 => {
                let encoded = if matches!(action.kind, 17 | 18) { parameter } else { action.parameters[6].as_str() };
                let waypoint = if matches!(action.kind, 17 | 18) { encoded.parse::<i32>().ok() } else { decode_waypoint(encoded) };
                let cell = waypoint.and_then(|id| data.waypoints.iter().find(|(other, _)| *other == id).map(|(_, cell)| *cell));
                let (Some(waypoint), Some(cell)) = (waypoint, cell) else {
                    state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::MissingWaypoint { waypoint: encoded.into() },
                        "A mission action references a missing or invalid waypoint.");
                    return;
                };
                match action.kind {
                    17 => {
                        let level = self.map.level(cell.0, cell.1) as i32 + if self.map.bridged(cell.0, cell.1) { 4 } else { 0 };
                        // Action17 preprojects once; RevealArea2 projects Z a
                        // second time. Keep the caller's origin for occlusion.
                        let caller_cell = (cell.0 - level / 2, cell.1 - level / 2);
                        self.map.reveal_with_height(data.player_house, caller_cell, level, data.reveal_radius, data.reveal_by_height);
                        state.outputs.push(MissionOutput::Reveal { waypoint, cell });
                    }
                    18 => {
                        if self.map.zones().is_none() {
                            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::RevealGeometryUnverified { action: 18, feature: "MissingMovementZones".into() },
                                "Reveal Zone needs the map's Crusher movement zones; no replacement circle or whole-map reveal was invented.");
                            return;
                        }
                        // Original GetMovementZoneType uses Crusher=1 and
                        // isBridge=false for both seed and every visited Cell.
                        // It compares the zone IDs directly, even ID1/ffff.
                        let zone = self.map.zone_at(cell, 1);
                        for y in 0..self.map.height {
                            for x in 0..self.map.width {
                                // CellIteratorNext explicitly requires
                                // IsWithinUsableArea(cell,true), including
                                // LocalSize and this cell's height.
                                if self.map.in_playfield(x, y) && self.map.zone_at((x, y), 1) == zone {
                                    // Native18 halves Level before computing
                                    // projection (Level/4) and height. Unlike17
                                    // it does not add the bridge's four levels.
                                    let level = self.map.level(x, y) as i32 / 2;
                                    let caller_cell = (x - level / 2, y - level / 2);
                                    self.map.reveal_with_height(data.player_house, caller_cell, level, 2, data.reveal_by_height);
                                }
                            }
                        }
                        state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::RevealGeometryUnverified { action: 18, feature: "CellIteratorAndRevealRaster".into() },
                            "Reveal Zone uses the existing Crusher-zone IDs, usable-area cells and radius2 per matching cell. Original missing-cell storage, bridge-zone construction and paired RevealArea2 raster details still require differential geometry verification.");
                        state.outputs.push(MissionOutput::Reveal { waypoint, cell });
                    }
                    kind => {
                        let Ok(value) = parameter.parse::<i32>() else {
                            state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::InvalidParameter { kind, index },
                                "A camera speed or radar-event kind is not an integer.");
                            return;
                        };
                        state.outputs.push(if kind == 48 {
                            MissionOutput::Camera { speed: value, waypoint, cell }
                        } else {
                            MissionOutput::Radar { kind: value, waypoint, cell }
                        });
                    }
                }
            }
            _ => state.diagnose(self.tick, Some(&trigger.id), DiagnosticKind::UnsupportedAction { kind: action.kind, index },
                "An active mission action is not implemented; it has not been silently treated as successful."),
        }
    }
}

/// Original action waypoint encoding: A=0, Z=25, AA=26. Numeric strings are
/// accepted for callers constructing diagnostic scenarios.
pub fn decode_waypoint(encoded: &str) -> Option<i32> {
    if let Ok(value) = encoded.parse::<i32>() {
        return (value >= 0).then_some(value);
    }
    if encoded.is_empty() {
        return None;
    }
    let mut number = 0i32;
    for character in encoded.bytes() {
        if !character.is_ascii_alphabetic() {
            return None;
        }
        number = number
            .checked_mul(26)?
            .checked_add((character.to_ascii_uppercase() - b'A') as i32 + 1)?;
    }
    number.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{House, Map};

    fn field() -> World {
        let mut map = Map::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                map.set_cell(x, y, true, 0);
            }
        }
        let mut world = World::new(map, None);
        world.houses = vec![
            House::default(),
            House {
                human: true,
                ..House::default()
            },
        ];
        world
    }

    fn trigger(id: &str, kind: i32, parameter: i32, actions: Vec<ActionDef>) -> TriggerDef {
        TriggerDef {
            id: id.into(),
            events: vec![EventDef {
                kind,
                parameter,
                ..EventDef::default()
            }],
            actions,
            ..TriggerDef::default()
        }
    }

    fn mission(triggers: Vec<TriggerDef>) -> MissionData {
        let tags = triggers
            .iter()
            .map(|trigger| TagDef {
                id: format!("tag-{}", trigger.id),
                trigger: trigger.id.clone(),
                ..TagDef::default()
            })
            .collect();
        MissionData {
            id: "timer-probe".into(),
            player_house: 1,
            houses: vec![HouseBinding {
                name: "Player House".into(),
                country: "Player".into(),
                house: 1,
            }],
            country_names: vec!["Neutral".into(), "Player".into()],
            triggers,
            tags,
            ..MissionData::default()
        }
    }

    fn fired(world: &World, id: &str) -> u64 {
        world
            .mission_state
            .as_ref()
            .unwrap()
            .triggers
            .iter()
            .filter(|trigger| trigger.id == id)
            .map(|trigger| trigger.fire_count)
            .sum()
    }

    fn diplomacy_field(yuri: bool, kind: i32, parameter: &str) -> World {
        let mut world = field();
        world.houses = vec![House::default(); 4];
        world.houses[1].human = true;
        world.houses[2].ai = Some(crate::ai::Brain::default());
        let mut definition = trigger("diplomacy", 8, 0, vec![ActionDef::with_parameter(kind, parameter)]);
        definition.country = "Actor".into();
        let mut data = mission(vec![definition]);
        data.yuri = yuri;
        data.country_names = (0..100).map(|index| format!("Country{index}")).collect();
        data.country_names[5] = "Actor".into();
        data.country_names[13] = "Player".into();
        data.houses = [(2, 5, "Actor"), (1, 13, "Player"), (3, 42, "Other"), (0, 99, "Neutral")].into_iter().map(|(house, country, name)| {
            world.houses[house as usize].country = country;
            HouseBinding { house, name: name.into(), country: name.into() }
        }).collect();
        data.passive_houses.push(0);
        world.attach_mission(data).unwrap();
        world
    }

    fn execute_diplomacy(world: &mut World, action: ActionDef) -> bool {
        let data = world.mission_data.take().unwrap();
        let mut state = world.mission_state.take().unwrap();
        let result = world.mission_diplomacy_action(&data, &mut state, &data.triggers[0], 0, 0, &action);
        world.mission_data = Some(data);
        world.mission_state = Some(state);
        result
    }

    #[test]
    fn campaign_diplomacy_initializes_original_order_and_enemy_for_a_player_without_brain() {
        for yuri in [false, true] {
            let mut world = diplomacy_field(yuri, 38, "13suffix");
            for (house, nodes) in [(2, vec![1, 3, 0]), (1, vec![2, 3, 0]), (3, vec![2, 1, 0]), (0, vec![2, 1, 3])] {
                let diplomacy = world.mission_diplomacy(house).unwrap();
                assert_eq!(diplomacy.enemy_index, -1);
                assert_eq!(diplomacy.anger.iter().map(|node| node.house).collect::<Vec<_>>(), nodes);
                assert!(diplomacy.anger.iter().all(|node| node.score == 0));
            }
            // The event-populated runtime House does not supply FireActions' actor.
            world.mission_state.as_mut().unwrap().triggers[0].house = Some(3);
            let random = (world.random.clone(), world.campaign_random.clone());
            world.run_mission();
            assert_eq!(world.mission_diplomacy(2).unwrap().enemy_index, 1);
            assert_eq!(world.mission_diplomacy(1).unwrap().enemy_index, 0);
            assert_eq!(world.mission_diplomacy(3).unwrap().enemy_index, -1);
            assert_eq!(world.enemy_house(2), Some(1));
            assert_eq!(world.enemy_house(1), Some(2));
            assert_eq!(world.houses[2].ai.as_ref().unwrap().enemy, Some(1));
            assert!(world.houses[1].ai.is_none());
            assert_eq!(random, (world.random.clone(), world.campaign_random.clone()));
            assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().all(|diagnostic| !matches!(diagnostic.kind, DiagnosticKind::UnsupportedAction { kind: 38, .. })));
        }
    }

    #[test]
    fn make_enemy_preserves_original_selection_before_alliance_clear_and_make_ally_reselects() {
        let mut world = diplomacy_field(true, 38, "13");
        world.houses[2].allies = 1 << 1;
        world.houses[1].allies = 1 << 2;
        world.run_mission();
        assert_eq!(world.mission_diplomacy(2).unwrap().enemy_index, -1);
        assert_eq!(world.mission_diplomacy(1).unwrap().enemy_index, 0);
        assert!(!world.allied(2, 1) && !world.allied(1, 2));
        assert!(execute_diplomacy(&mut world, ActionDef::with_parameter(37, "13")));
        assert!(world.allied(2, 1) && world.allied(1, 2));
        assert_eq!(world.mission_diplomacy(2).unwrap().enemy_index, -1);
        assert_eq!(world.mission_diplomacy(1).unwrap().enemy_index, -1);
        assert_eq!(world.mission_diplomacy(2).unwrap().anger[0].score, 0);
        assert_eq!(world.mission_diplomacy(1).unwrap().anger[0].score, 0);
    }

    #[test]
    fn yr_runtime_house_sentinel_is_8997_and_missing_targets_do_not_invent_enemies() {
        let mut world = diplomacy_field(true, 38, "8997");
        world.mission_state.as_mut().unwrap().triggers[0].house = Some(3);
        world.run_mission();
        assert_eq!(world.enemy_house(2), Some(3));
        assert_eq!(world.enemy_house(3), Some(2));
        assert_eq!(world.enemy_house(1), None);
        for (yuri, raw) in [(false, "999"), (true, "9000"), (true, "-1"), (true, "4475")] {
            let mut world = diplomacy_field(yuri, 38, raw);
            world.mission_state.as_mut().unwrap().triggers[0].house = Some(1);
            let before = world.mission_state.as_ref().unwrap().diplomacy.clone();
            assert!(!execute_diplomacy(&mut world, ActionDef::with_parameter(38, raw)));
            assert_eq!(world.mission_state.as_ref().unwrap().diplomacy, before);
            assert!(world.houses[2].ai.as_ref().unwrap().enemy.is_none());
        }
        let mut world = diplomacy_field(false, 38, "-1");
        let before = world.mission_state.as_ref().unwrap().diplomacy.clone();
        assert!(execute_diplomacy(&mut world, ActionDef::with_parameter(38, "-1")));
        assert_eq!(world.mission_state.as_ref().unwrap().diplomacy, before);
    }

    #[test]
    fn campaign_anger_sink_uses_saved_order_stored_defeated_and_wrapping_without_inserting_nodes() {
        let mut world = diplomacy_field(true, 38, "13");
        assert!(world.units.is_empty() && world.buildings.is_empty());
        world.adjust_anger(2, 10, 3);
        world.adjust_anger(2, 10, 1);
        assert_eq!(world.enemy_house(2), Some(1), "tie follows original registration order, not runtime IDs");
        world.houses[1].defeated = true;
        world.adjust_anger(2, 0, 1);
        assert_eq!(world.enemy_house(2), Some(3), "stored defeated flag excludes an otherwise higher node");
        world.adjust_anger(1, i32::MAX, 2);
        world.adjust_anger(1, 1, 2);
        assert_eq!(world.mission_diplomacy(1).unwrap().anger[0].score, i32::MIN);
        assert_eq!(world.enemy_house(1), None);
        let count = world.mission_diplomacy(2).unwrap().anger.len();
        world.adjust_anger(2, 100, 2);
        assert_eq!(world.mission_diplomacy(2).unwrap().anger.len(), count, "normal ctor has no self node");
        let views = world.diplomacy_views(world.mission_data.as_ref().unwrap());
        assert_eq!(views.iter().map(|view| view.house).collect::<Vec<_>>(), vec![2, 1, 3, 0]);
        assert_eq!(world.houses[2].ai.as_ref().unwrap().anger[3], 10);
    }

    #[test]
    fn diplomacy_state_roundtrips_in_current_snapshot_with_ai_projection_and_future_actions() {
        let mut world = diplomacy_field(true, 38, "13");
        world.run_mission();
        world.adjust_anger(2, i32::MAX, 3);
        let saved = world.snapshot();
        assert_eq!(u32::from_le_bytes(saved[..4].try_into().unwrap()), crate::snapshot::FORMAT);
        let mut restored = diplomacy_field(true, 38, "13");
        restored.restore(&saved).unwrap();
        assert_eq!(world.snapshot(), restored.snapshot());
        assert_eq!(restored.mission_diplomacy(1).unwrap().enemy_index, 0);
        for action in [ActionDef::with_parameter(37, "13"), ActionDef::with_parameter(38, "42"), ActionDef::with_parameter(38, "13")] {
            assert_eq!(execute_diplomacy(&mut world, action.clone()), execute_diplomacy(&mut restored, action));
            world.adjust_anger(1, 7, 2);
            restored.adjust_anger(1, 7, 2);
            assert_eq!(world.state_hash(), restored.state_hash());
        }
        let mut old = saved;
        old[..4].copy_from_slice(&(crate::snapshot::FORMAT - 1).to_le_bytes());
        assert!(diplomacy_field(true, 38, "13").restore(&old).is_err());
    }

    fn team_world(script: Vec<(i32, i32)>) -> World {
        let mut world = field();
        world.houses.push(House::default());
        world.unit_types.push(crate::UnitType {
            stats: crate::UnitStats {
                kind: 1,
                strength: 100,
                speed: 6,
                ..crate::UnitStats::default()
            },
            ..crate::UnitType::default()
        });
        world.team_data.task_forces.push(vec![(1, 0)]);
        world.team_data.scripts.push(script);
        world.team_data.team_types.push(crate::teams::TeamType {
            name: "Map team".into(),
            max: 0,
            members_recruitable: true,
            ..crate::teams::TeamType::default()
        });
        world
    }

    fn team_mission() -> MissionData {
        let mut data = mission(vec![trigger(
            "create",
            8,
            0,
            vec![ActionDef::with_parameter(4, "map-team")],
        )]);
        data.houses.push(HouseBinding {
            name: "Enemy House".into(),
            country: "Enemy Country".into(),
            house: 2,
        });
        data.team_types = vec![("map-team".into(), 0)];
        data.team_definitions = vec![TeamDef {
            kind: 0,
            house_country: "Enemy Country".into(),
            group: -2,
            ..TeamDef::default()
        }];
        data
    }

    fn placed(id: u32, group: i32) -> ObjectBinding {
        ObjectBinding {
            section: "Units".into(),
            id: format!("row-{id}"),
            tag: None,
            object: crate::combat::Aim::Unit(id),
            group,
            initial_mission: "Guard".into(),
        }
    }

    fn tagged_reinforcements(persistence: u8, event: i32, count: u32) -> World {
        let mut world = team_world(vec![(11, 5)]);
        world.team_data.task_forces[0] = vec![(count, 0)];
        let mut data = team_mission();
        data.triggers[0].actions[0].kind = 7;
        data.team_definitions[0].waypoint = Some(0);
        data.team_definitions[0].tag = Some("death-tag".into());
        data.waypoints.push((0, (4, 4)));
        data.triggers.push(trigger("death", event, 12345, vec![ActionDef::with_parameter(19, "DeathText")]));
        data.tags.push(TagDef { id: "death-tag".into(), trigger: "death".into(), persistence, ..TagDef::default() });
        world.attach_mission(data).unwrap();
        world
    }

    #[test]
    fn team_tags_are_distinct_instances_and_object_only_definitions_stay_dormant() {
        let mut world = tagged_reinforcements(0, 48, 2);
        assert!(!world.mission_state.as_ref().unwrap().tags[1].instantiated);
        assert_eq!(world.mission_state.as_ref().unwrap().triggers.len(), 1);
        let action = world.mission_data.as_ref().unwrap().triggers[0].actions[0].clone();
        world.mission_data.as_mut().unwrap().triggers[0].actions.push(action);
        world.run_mission();
        let first = world.teams[0].scenario_tag.unwrap();
        let second = world.teams[1].scenario_tag.unwrap();
        assert_ne!(first, second);
        assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(first), 2);
        assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(second), 2);
        assert!(!world.mission_state.as_ref().unwrap().tags[first].periodic);
        let victim = world.teams[0].members[0];
        world.kill_unit_for_tests(victim);
        let state = world.mission_state.as_ref().unwrap();
        assert_eq!(fired(&world, "death"), 1);
        assert!(state.tags[first].pending_delete);
        assert!(!state.tags[first].destroyed, "Notice precedes the deferred Tag destructor");
        assert_eq!(state.tag_refs(first), 0, "Notice detaches every remaining live member");
        assert_eq!(state.tag_refs(second), 2, "a separate Team constructor owns a separate Tag");
        world.finish_mission_deletions();
        assert!(world.mission_state.as_ref().unwrap().tags[first].destroyed);
    }

    #[test]
    fn map_and_house_registration_have_separate_notification_sources() {
        let mut world = field();
        let mut data = mission(vec![trigger("map", 24, 0, Vec::new()), trigger("house", 3, 0, Vec::new())]);
        data.triggers[1].country = "Player".into();
        world.attach_mission(data).unwrap();
        let state = world.mission_state.as_ref().unwrap();
        assert!(state.tags.iter().all(|tag| tag.instantiated && !tag.periodic));
        world.run_mission();
        assert_eq!(fired(&world, "map"), 0);
        assert_eq!(fired(&world, "house"), 0);
        assert_eq!(world.mission_state.as_ref().unwrap().diagnostics.iter().filter(|diagnostic| matches!(diagnostic.kind, DiagnosticKind::UnsupportedEvent { kind: 24 | 3, .. })).count(), 1);
        world.tick = 8;
        world.run_house_mission_tags(1);
        assert_eq!(fired(&world, "house"), 0);
        assert_eq!(world.mission_state.as_ref().unwrap().diagnostics.iter().filter(|diagnostic| matches!(diagnostic.kind, DiagnosticKind::UnsupportedEvent { kind: 24 | 3, .. })).count(), 2);
    }

    #[test]
    fn tag_notice_clears_the_teams_pointer_before_later_recruitment() {
        let mut world = tagged_reinforcements(0, 48, 2);
        world.run_mission();
        let team = world.teams[0].id;
        let tag = world.teams[0].scenario_tag.unwrap();
        world.kill_unit_for_tests(world.teams[0].members[0]);
        assert_eq!(world.teams[0].scenario_tag, None);
        let recruit = world.add_unit((10, 10), 0, world.unit_types[0].stats, 2);
        assert_eq!(world.recruit_team(team).unwrap(), 1);
        assert!(world.teams[0].members.contains(&recruit));
        assert_eq!(world.mission_state.as_ref().unwrap().object_tag(crate::combat::Aim::Unit(recruit)), None);
        assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), 0);
    }

    #[test]
    fn last_member_tag_waits_for_all_members_and_replays_midway_from_snapshot() {
        let mut world = tagged_reinforcements(1, 48, 3);
        world.run_mission();
        let members = world.teams[0].members.clone();
        let tag = world.teams[0].scenario_tag.unwrap();
        world.kill_unit_for_tests(members[0]);
        assert_eq!(fired(&world, "death"), 0);
        assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), 2);
        let saved = world.snapshot();
        let mut restored = tagged_reinforcements(1, 48, 3);
        restored.restore(&saved).unwrap();
        for candidate in [&mut world, &mut restored] {
            candidate.kill_unit_for_tests(members[1]);
            assert_eq!(fired(candidate, "death"), 0);
            assert_eq!(candidate.mission_state.as_ref().unwrap().tag_refs(tag), 1);
            candidate.kill_unit_for_tests(members[2]);
            assert_eq!(fired(candidate, "death"), 1, "Event48 ignores the arbitrary event Value");
            assert_eq!(candidate.mission_state.as_ref().unwrap().tag_refs(tag), 0);
            assert!(candidate.mission_state.as_ref().unwrap().tags[tag].pending_delete);
            candidate.tick();
        }
        assert_eq!(world.state_hashes(), restored.state_hashes());
        assert_eq!(world.take_mission_outputs(), restored.take_mission_outputs());
    }

    #[test]
    fn on_trans_only_attaches_the_team_tag_only_to_passenger_carriers() {
        let mut world = tagged_reinforcements(1, 48, 1);
        world.unit_types[0].passengers = 4;
        world.unit_types.push(crate::UnitType { stats: crate::UnitStats { kind: 2, ..world.unit_types[0].stats }, ..crate::UnitType::default() });
        world.team_data.task_forces[0].push((1, 1));
        world.mission_data.as_mut().unwrap().team_definitions[0].properties.push(("OnTransOnly".into(), "yes".into()));
        world.run_mission();
        let tag = world.teams[0].scenario_tag.unwrap();
        let carrier = world.units.iter().find(|unit| unit.stats.kind == 1).unwrap().id;
        let other = world.units.iter().find(|unit| unit.stats.kind == 2).unwrap().id;
        assert_eq!(world.mission_state.as_ref().unwrap().tags[tag].objects, vec![crate::combat::Aim::Unit(carrier)]);
        world.kill_unit_for_tests(other);
        assert_eq!(fired(&world, "death"), 0);
        world.kill_unit_for_tests(carrier);
        assert_eq!(fired(&world, "death"), 1);
    }

    #[test]
    fn attacker_destruction_raises_seven_before_48_and_rereads_the_attachment() {
        for has_attacker in [false, true] {
            let mut world = tagged_reinforcements(0, 7, 1);
            let data = world.mission_data.as_mut().unwrap();
            data.triggers[1].linked = Some("destroyed48".into());
            data.triggers.push(trigger("destroyed48", 48, 0, Vec::new()));
            // Definitions changed before attachment in production; rebuild the
            // fixture so immutable rules and runtime instances agree.
            let data = world.mission_data.take().unwrap();
            world.attach_mission(data).unwrap();
            world.warheads.push(crate::combat::Warhead::default());
            world.run_mission();
            let victim = crate::combat::Aim::Unit(world.teams[0].members[0]);
            let attacker = has_attacker.then(|| crate::combat::Aim::Unit(world.add_unit((8, 8), 0, world.unit_types[0].stats, 1)));
            assert!(world.receive_damage(victim, 1000, 0, 0, attacker));
            assert_eq!(fired(&world, "death"), u64::from(has_attacker));
            assert_eq!(fired(&world, "destroyed48"), u64::from(!has_attacker), "7's Notice clears the tag before the later 48 source");
        }
    }

    #[test]
    fn persistent_death_event_latches_until_a_later_global_timer_condition() {
        let mut world = field();
        let victim = world.add_unit((4, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
        let mut data = mission(vec![trigger("death-and-timer", 48, -1, Vec::new())]);
        data.triggers[0].events.push(EventDef { kind: 13, parameter: 1, ..EventDef::default() });
        data.tags[0].persistence = 2;
        let mut binding = placed(victim, -1);
        binding.tag = Some(data.tags[0].id.clone());
        data.objects.push(binding);
        world.attach_mission(data).unwrap();
        world.kill_unit_for_tests(victim);
        assert_eq!(fired(&world, "death-and-timer"), 0);
        assert_eq!(world.mission_state.as_ref().unwrap().triggers[0].occurred, vec![true, false]);
        for _ in 0..15 { world.tick(); }
        assert_eq!(fired(&world, "death-and-timer"), 1);
        assert_eq!(world.mission_state.as_ref().unwrap().triggers[0].occurred, vec![true, false], "only the destruction condition is latched");
        for _ in 0..15 { world.tick(); }
        assert_eq!(fired(&world, "death-and-timer"), 2);
    }

    #[test]
    fn team_member_release_retains_human_tags_and_detaches_computer_tags() {
        for human in [false, true] {
            let mut world = tagged_reinforcements(1, 48, 2);
            world.houses[2].human = human;
            world.team_data.scripts[0] = vec![(8, 3)];
            world.run_mission();
            let tag = world.teams[0].scenario_tag.unwrap();
            for _ in 0..3 { world.run_teams(); }
            assert!(world.units.iter().all(|unit| unit.team.is_none()));
            assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), if human { 2 } else { 0 });
            assert_eq!(world.mission_state.as_ref().unwrap().tags[tag].destroyed, !human);
            assert_eq!(fired(&world, "death"), 0, "team release is not a destruction source");
        }
    }

    #[test]
    fn linked_attached_triggers_follow_native_head_insertion_order() {
        let mut world = tagged_reinforcements(2, 48, 1);
        let data = world.mission_data.as_mut().unwrap();
        data.triggers[1].linked = Some("second".into());
        data.triggers.push(trigger("second", 48, 0, Vec::new()));
        data.triggers[2].linked = Some("third".into());
        data.triggers.push(trigger("third", 48, 0, Vec::new()));
        let data = world.mission_data.take().unwrap();
        world.attach_mission(data).unwrap();
        world.run_mission();
        world.take_mission_outputs();
        world.kill_unit_for_tests(world.teams[0].members[0]);
        let order: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::TriggerFired { trigger_id, .. } = output { Some(trigger_id) } else { None }).collect();
        assert_eq!(order, vec!["third", "second", "death"]);
    }

    #[test]
    fn enable_finds_a_trigger_destroyed_earlier_in_the_same_tag_notification() {
        let mut world = tagged_reinforcements(0, 48, 1);
        let data = world.mission_data.as_mut().unwrap();
        data.triggers[1].linked = Some("earlier".into());
        data.triggers[1].actions = vec![ActionDef::with_parameter(53, "earlier")];
        data.triggers.push(trigger("earlier", 48, 0, Vec::new()));
        let data = world.mission_data.take().unwrap();
        world.attach_mission(data).unwrap();
        world.run_mission();
        world.kill_unit_for_tests(world.teams[0].members[0]);
        let state = world.mission_state.as_ref().unwrap();
        let earlier = state.triggers.iter().find(|trigger| trigger.id == "earlier").unwrap();
        assert!(earlier.destroyed && earlier.enabled);
        assert_eq!(earlier.reset_generation, 2, "Enable still resets a queued Trigger before its destructor");
        assert!(!state.diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::MissingRuntimeTrigger { target } if target == "earlier")));
    }

    #[test]
    fn deployment_and_packing_transfer_one_attachment_without_raising_destroyed() {
        let mut world = field();
        let stats = crate::UnitStats { kind: 1, strength: 100, deploys_into: Some(0), ..crate::UnitStats::default() };
        world.unit_types.push(crate::UnitType { stats, deploy_facing: Some(0), ..crate::UnitType::default() });
        world.building_types.push(crate::BuildingType {
            strength: 200, size: (1, 1), undeploys_into: Some(stats), deploy_facing: Some(0), ..crate::BuildingType::default()
        });
        let original = world.add_unit((5, 5), 0, stats, 1);
        let mut data = mission(vec![trigger("destroyed", 48, 0, Vec::new())]);
        data.tags[0].persistence = 1;
        let mut binding = placed(original, -1);
        binding.tag = Some(data.tags[0].id.clone());
        data.objects.push(binding);
        world.attach_mission(data).unwrap();
        world.order(crate::Order::Deploy { unit: original });
        world.tick();
        assert!(world.unit(original).is_none());
        let building = world.buildings[0].id;
        assert_eq!(world.mission_state.as_ref().unwrap().tags[0].objects, vec![crate::combat::Aim::Building(building)]);
        assert_eq!(fired(&world, "destroyed"), 0);
        world.order(crate::Order::Undeploy { building, x: 8, y: 8 });
        assert!(world.buildings.is_empty());
        let replacement = world.units[0].id;
        assert_ne!(replacement, original);
        assert_eq!(world.mission_state.as_ref().unwrap().tags[0].objects, vec![crate::combat::Aim::Unit(replacement)]);
        assert_eq!(fired(&world, "destroyed"), 0);
        world.kill_unit_for_tests(replacement);
        assert_eq!(fired(&world, "destroyed"), 1);
    }

    #[test]
    fn normal_building_sale_raises_48_and_construction_or_direct_erase_does_not() {
        for path in ["sale", "construction", "erase"] {
            let mut world = field();
            world.building_types.push(crate::BuildingType { strength: 100, cost: 1000, size: (1, 1), ..crate::BuildingType::default() });
            let building = world.add_map_building(1, 0, (5, 5), 100);
            let mut data = mission(vec![trigger("destroyed48", 48, 0, Vec::new()), trigger("destroyed7", 7, 0, Vec::new())]);
            data.triggers[0].linked = Some("destroyed7".into());
            data.tags.truncate(1);
            data.tags[0].persistence = 1;
            data.objects.push(ObjectBinding { section: "Structures".into(), id: "0".into(), tag: Some(data.tags[0].id.clone()), object: crate::combat::Aim::Building(building), group: -1, initial_mission: "Guard".into() });
            world.attach_mission(data).unwrap();
            match path {
                "sale" => world.order(crate::Order::Sell { building }),
                "erase" => { world.demolish(0); }
                _ => {
                    world.buildings[0].buildup = Some(crate::Buildup { stage: 0, count: 3, rate: 1, next: 1, packing: None, selling: false });
                    for _ in 0..4 { world.tick(); }
                    assert!(world.buildings[0].buildup.is_none());
                }
            }
            assert_eq!(fired(&world, "destroyed48"), u64::from(path == "sale"));
            assert_eq!(fired(&world, "destroyed7"), 0);
            if path == "sale" { assert!(world.buildings.is_empty()); }
        }
    }

    #[test]
    fn c4_uses_the_registered_applying_soldier_pointer_including_off_map_cargo() {
        for source in ["map", "cargo", "patient", "expired"] {
            let mut world = field();
            let soldier = world.add_unit((2, 2), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
            world.building_types.push(crate::BuildingType { strength: 100, size: (1, 1), ..crate::BuildingType::default() });
            let building = world.add_map_building(0, 0, (5, 5), 256);
            world.buildings[0].c4 = Some((1, soldier));
            if source == "expired" {
                world.kill_unit_for_tests(soldier);
            } else if source != "map" {
                let stored = world.lift(0);
                if source == "cargo" {
                    world.add_unit((7, 7), 0, crate::UnitStats::default(), 1);
                    world.units[0].cargo.push(stored);
                } else { world.buildings[0].patients.push((100, stored)); }
            }
            let mut data = mission(vec![trigger("seven", 7, 0, Vec::new()), trigger("48", 48, 0, Vec::new())]);
            data.triggers[0].linked = Some("48".into());
            data.tags.truncate(1);
            data.objects.push(ObjectBinding { section: "Structures".into(), id: "0".into(), tag: Some(data.tags[0].id.clone()), object: crate::combat::Aim::Building(building), group: -1, initial_mission: "Guard".into() });
            world.attach_mission(data).unwrap();
            world.tick = 1;
            world.detonate_charges();
            assert!(world.buildings.is_empty());
            assert_eq!(fired(&world, "seven"), u64::from(source != "expired"), "C4 source {source}");
            assert_eq!(fired(&world, "48"), u64::from(source == "expired"), "earlier7 clears the tag before48");
        }
    }

    #[test]
    fn grinder_consumption_raises_48_but_bioreactor_absorption_retains_the_attachment() {
        for infantry in [false, true] { for grind in [false, true] {
            let mut world = field();
            world.unit_types.push(crate::UnitType { cost: 100, soylent: Some(37), size: 1, ..crate::UnitType::default() });
            world.building_types.push(crate::BuildingType {
                strength: 100, size: (1, 1), grinding: grind, infantry_absorb: true, unit_absorb: true,
                extra_power: 100, passengers: 3, size_limit: 1, ..crate::BuildingType::default()
            });
            let building = world.add_map_building(1, 0, (5, 5), 256);
            let unit = world.add_unit((5, 5), 0, crate::UnitStats { kind: 1, strength: 100, infantry, ..crate::UnitStats::default() }, 1);
            world.units[0].capture = Some(building);
            let mut data = mission(vec![trigger("48", 48, 0, Vec::new()), trigger("seven", 7, 0, Vec::new())]);
            data.triggers[0].linked = Some("seven".into());
            data.tags.truncate(1);
            let mut binding = placed(unit, -1);
            binding.tag = Some(data.tags[0].id.clone());
            data.objects.push(binding);
            world.attach_mission(data).unwrap();
            let credits = world.houses[1].credits;
            world.enter_buildings();
            assert!(world.unit(unit).is_none());
            assert_eq!(fired(&world, "48"), u64::from(grind));
            assert_eq!(fired(&world, "seven"), 0);
            assert_eq!(world.houses[1].credits, credits + if grind { 37 } else { 0 });
            assert_eq!(world.buildings[0].occupants.len(), usize::from(!grind));
            assert_eq!(world.mission_state.as_ref().unwrap().tags[0].objects.len(), usize::from(!grind));
            if !grind { assert!(world.has_registered_mission_object(crate::combat::Aim::Unit(unit))); }
        }}
    }

    #[test]
    fn successful_engineer_and_spy_entries_raise_48_and_rejected_repair_does_not() {
        for entry in ["capture", "repair", "spy", "full"] {
            let mut world = field();
            world.building_types.push(crate::BuildingType {
                strength: 100, size: (1, 1), spyable: true,
                fighter: crate::combat::Fighter { capturable: true, ..crate::combat::Fighter::default() }, ..crate::BuildingType::default()
            });
            let owner = if matches!(entry, "capture" | "spy") { 0 } else { 1 };
            let building = world.add_map_building(owner, 0, (5, 5), if entry == "repair" { 128 } else { 256 });
            let unit = world.add_unit((5, 5), 0, crate::UnitStats { strength: 100, infantry: true,
                fighter: crate::combat::Fighter { engineer: entry != "spy", agent: entry == "spy", ..crate::combat::Fighter::default() }, ..crate::UnitStats::default() }, 1);
            world.units[0].capture = Some(building);
            let mut data = mission(vec![trigger("48", 48, 0, Vec::new())]);
            let mut binding = placed(unit, -1);
            binding.tag = Some(data.tags[0].id.clone());
            data.objects.push(binding);
            world.attach_mission(data).unwrap();
            world.enter_buildings();
            assert_eq!(fired(&world, "48"), u64::from(entry != "full"), "entry {entry}");
            assert_eq!(world.dying.iter().any(|(object, _)| *object == crate::combat::Aim::Unit(unit)), entry != "full");
            if entry == "capture" { assert_eq!(world.buildings[0].owner, 1); }
            if entry == "repair" { assert_eq!(world.buildings[0].health, 100); }
        }
    }

    #[test]
    fn must_transfer_shares_the_capturing_engineers_tag_before_its_last_reference_test() {
        for must_transfer in [false, true] {
            let mut world = field();
            world.building_types.push(crate::BuildingType { strength: 100, size: (1, 1), fighter: crate::combat::Fighter { capturable: true, ..crate::combat::Fighter::default() }, ..crate::BuildingType::default() });
            let building = world.add_map_building(0, 0, (5, 5), 256);
            let engineer = world.add_unit((5, 5), 0, crate::UnitStats { strength: 100, infantry: true,
                fighter: crate::combat::Fighter { engineer: true, ..crate::combat::Fighter::default() }, ..crate::UnitStats::default() }, 1);
            world.units[0].capture = Some(building);
            let mut data = mission(vec![trigger("48", 48, 0, Vec::new()), trigger("old-building-tag", 48, 0, Vec::new()), trigger("transfer-only", 0, 0, Vec::new())]);
            data.triggers[0].linked = Some("transfer-only".into());
            data.triggers[2].enabled = false;
            data.triggers[2].must_transfer = must_transfer;
            data.tags.truncate(2);
            data.tags[0].persistence = 1;
            let mut binding = placed(engineer, -1);
            binding.tag = Some(data.tags[0].id.clone());
            data.objects = vec![binding, ObjectBinding { section: "Structures".into(), id: "0".into(), tag: Some(data.tags[1].id.clone()), object: crate::combat::Aim::Building(building), group: -1, initial_mission: "Guard".into() }];
            world.attach_mission(data).unwrap();
            world.enter_buildings();
            assert_eq!(fired(&world, "48"), u64::from(!must_transfer), "shared building reference postpones the last-member trigger");
            if must_transfer {
                assert_eq!(world.mission_state.as_ref().unwrap().tags[0].objects, vec![crate::combat::Aim::Building(building)]);
                assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(1), 0, "building's old Tag was replaced");
            }
            world.kill(crate::combat::Aim::Building(building));
            assert_eq!(fired(&world, "48"), 1);
            assert_eq!(fired(&world, "old-building-tag"), u64::from(!must_transfer));
        }
    }

    #[test]
    fn ordinary_reinforcement_constructs_then_places_reverse_chain_at_the_same_waypoint() {
        let mut world = team_world(vec![(11, 5)]);
        world.unit_types.push(crate::UnitType { naval: true, stats: crate::UnitStats { kind: 2, strength: 75, ..world.unit_types[0].stats }, ..crate::UnitType::default() });
        world.team_data.task_forces[0] = vec![(2, 0), (1, 1)];
        let previous = world.add_unit((4, 4), 0, world.unit_types[0].stats, 2);
        let mut data = team_mission();
        data.triggers[0].actions[0].kind = 7;
        data.team_definitions[0].waypoint = Some(3);
        data.team_definitions[0].group = 17;
        data.team_definitions[0].properties.push(("VeteranLevel".into(), "3".into()));
        data.waypoints.push((3, (4, 4)));
        data.house_edges.push((2, 1));
        world.attach_mission(data).unwrap();
        let rng = (world.random, world.idle_random);
        world.run_mission();
        assert_eq!((world.random, world.idle_random), rng);
        assert_eq!(world.teams.len(), 1, "Max=0 does not gate direct reinforcement construction");
        let team = &world.teams[0];
        assert_eq!(team.house, 2, "TeamType owner takes precedence over trigger owner");
        assert_eq!(team.line, -1, "the reinforcement constructor does not execute its script");
        let members = team.members.clone();
        assert_eq!(members, vec![team.id + 3, team.id + 2, team.id + 1]);
        assert_eq!(members.iter().map(|&id| world.unit(id).unwrap().stats.kind).collect::<Vec<_>>(), vec![2, 1, 1]);
        for &id in &members {
            let unit = world.unit(id).unwrap();
            assert_eq!((unit.owner, unit.cell(), unit.facing, unit.veterancy, unit.initiated), (2, (4, 4), 64, 2.0, true));
            assert_eq!(world.unit_mission(id), Some(UnitMission::Guard));
            assert_eq!(world.mission_state.as_ref().unwrap().unit_groups.iter().find(|entry| entry.0 == id).unwrap().1, 17);
        }
        let at = world.map.index(4, 4).unwrap();
        assert_eq!(world.stacked_claims, vec![(at, vec![members[2] + 1, members[1] + 1, members[0] + 1, previous + 1])]);
        let others = world.others();
        assert_eq!(crate::holder(&world.map, &world.claims, &world.stacked_claims, &others, (4, 4), members[2], false).unwrap().id, members[1]);
        world.run_teams();
        assert_eq!(world.teams[0].line, 0);
    }

    #[test]
    fn action_80_uses_its_encoded_override_and_native_frame_selection_without_rng() {
        let order = [(-1, -1), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 1), (-1, 0), (1, 0)];
        for (frame, delta) in order.into_iter().enumerate() {
            let mut world = team_world(vec![(11, 5)]);
            world.add_unit((4, 4), 0, world.unit_types[0].stats, 2);
            let mut data = team_mission();
            data.triggers[0].actions[0].kind = 80;
            data.triggers[0].actions[0].parameters[6] = "C".into();
            data.waypoints = vec![(1, (2, 2)), (2, (4, 4))];
            data.house_edges.push((2, 2));
            data.team_definitions[0].waypoint = Some(1);
            world.attach_mission(data).unwrap();
            world.tick = frame as u64;
            let rng = (world.random, world.idle_random);
            world.run_mission();
            let unit = world.unit(world.teams[0].members[0]).unwrap();
            assert_eq!(unit.cell(), (4 + delta.0, 4 + delta.1));
            assert_eq!(unit.facing, 0, "explicit inside override does not use House.Edge");
            assert_eq!((world.random, world.idle_random), rng);
        }
    }

    #[test]
    fn action_80_flat_water_float_keeps_the_clear_seed_for_every_frame() {
        for frame in 0..16 {
            let mut world = team_world(vec![(11, 5)]);
            world.unit_types[0].naval = true;
            world.unit_types[0].stats.speed_type = crate::SpeedType::Float;
            for multipliers in &mut world.map.lands.0 { multipliers[crate::SpeedType::Float as usize] = 0; }
            world.map.lands.0[crate::Land::Water as usize][crate::SpeedType::Float as usize] = 100;
            for y in 0..16 { for x in 0..16 { world.map.set_land(x, y, crate::Land::Water); } }
            world.team_data.task_forces[0] = vec![(2, 0)];
            let mut data = team_mission();
            data.triggers[0].actions[0].kind = 80;
            data.triggers[0].actions[0].parameters[6] = "A".into();
            data.waypoints.push((0, (4, 4)));
            world.attach_mission(data).unwrap();
            world.tick = frame;
            let rng = (world.random, world.idle_random);
            world.run_mission();
            assert_eq!(world.units.len(), 2);
            assert!(world.units.iter().all(|unit| unit.cell() == (4, 4) && unit.stats.speed_type == crate::SpeedType::Float));
            assert_eq!((world.random, world.idle_random), rng);
        }
    }

    #[test]
    fn reinforcement_infantry_force_the_center_without_a_three_soldier_cap_or_rng() {
        fn setup() -> World {
            let mut world = team_world(vec![(11, 5)]);
            world.unit_types[0].stats.infantry = true;
            world.unit_types[0].stats.speed_type = crate::SpeedType::Foot;
            world.team_data.task_forces[0] = vec![(5, 0)];
            let mut data = team_mission();
            data.triggers[0].actions[0].kind = 7;
            data.team_definitions[0].waypoint = Some(0);
            data.waypoints.push((0, (4, 4)));
            world.attach_mission(data).unwrap();
            world
        }
        let mut world = setup();
        let rng = (world.random, world.idle_random);
        world.run_mission();
        assert_eq!((world.random, world.idle_random), rng);
        assert_eq!(world.units.len(), 5);
        assert!(world.units.iter().all(|unit| unit.x == 4 * 256 + 128 && unit.y == 4 * 256 + 128 && unit.initiated));
        let index = world.map.index(4, 4).unwrap();
        assert_eq!((world.claims[index], world.spots[index], world.spot_house[index]), (0, 1, 2));
        world.teams.clear();
        for unit in &mut world.units { unit.team = None; }
        let leaving = world.units[0].id;
        world.order(crate::Order::Move { unit: leaving, x: 7, y: 4 });
        let snapshot = world.snapshot();
        let mut restored = setup();
        restored.restore(&snapshot).unwrap();
        for candidate in [&mut world, &mut restored] {
            candidate.tick();
            assert!(candidate.unit(leaving).unwrap().head_to.is_some());
            // Native occupation is a bit, not a reference count. The first
            // soldier's departure clears slot0 even with four others there.
            assert_eq!(candidate.spots[index] & 1, 0);
            assert_eq!(candidate.spot_house[index], u8::MAX);
            assert_eq!(candidate.units.iter().filter(|unit| unit.id != leaving && unit.cell() == (4, 4)).count(), 4);
            for _ in 0..100 { candidate.tick(); }
            assert_eq!(candidate.unit(leaving).unwrap().cell(), (7, 4));
        }
        assert_eq!(world.state_hash(), restored.state_hash());
    }

    #[test]
    fn mixed_reinforcement_keeps_vehicle_claims_and_forced_infantry_center_separate() {
        let mut world = team_world(vec![(11, 5)]);
        world.unit_types.push(crate::UnitType { stats: crate::UnitStats { infantry: true, kind: 2, speed_type: crate::SpeedType::Foot, ..world.unit_types[0].stats }, ..crate::UnitType::default() });
        world.team_data.task_forces[0] = vec![(1, 0), (5, 1)];
        let mut data = team_mission();
        data.triggers[0].actions[0].kind = 7;
        data.team_definitions[0].waypoint = Some(0);
        data.waypoints.push((0, (4, 4)));
        world.attach_mission(data).unwrap();
        world.run_mission();
        let vehicle = world.units.iter().find(|unit| !unit.stats.infantry).unwrap().id;
        let index = world.map.index(4, 4).unwrap();
        assert_eq!((world.units.len(), world.claims[index], world.spots[index]), (6, vehicle + 1, 1));
        assert!(world.stacked_claims.is_empty());
    }

    #[test]
    fn pure_infantry_near_a_building_stops_at_the_unreconstructed_special_insertion() {
        let mut world = team_world(vec![(11, 5)]);
        world.unit_types[0].stats.infantry = true;
        world.building_types.push(crate::BuildingType { strength: 100, size: (1, 1), ..crate::BuildingType::default() });
        world.add_map_building(2, 0, (5, 4), 100);
        let mut data = team_mission();
        data.triggers[0].actions[0].kind = 7;
        data.team_definitions[0].waypoint = Some(0);
        data.waypoints.push((0, (4, 4)));
        world.attach_mission(data).unwrap();
        world.run_mission();
        assert!(world.units.is_empty() && world.teams.is_empty());
        assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::ReinforcementUnverified { feature, .. } if feature == "InfantryBuildingInsertion")));
    }

    #[test]
    fn action_80_nearby_clearance_sees_forced_center_infantry_occupation() {
        let mut world = team_world(vec![(11, 5)]);
        world.unit_types[0].stats.infantry = true;
        world.unit_types[0].stats.speed_type = crate::SpeedType::Foot;
        let mut data = team_mission();
        data.triggers[0].actions[0].kind = 7;
        let mut second = ActionDef::with_parameter(80, "map-team");
        second.parameters[6] = "A".into();
        data.triggers[0].actions.push(second);
        data.team_definitions[0].waypoint = Some(0);
        data.waypoints.push((0, (4, 4)));
        world.attach_mission(data).unwrap();
        world.run_mission();
        assert_eq!(world.units.len(), 2);
        assert_eq!(world.units[0].cell(), (4, 4));
        assert_eq!(world.units[1].cell(), (3, 3), "frame0 selects NW from the native radius1 order");
    }

    #[test]
    fn reinforcement_stack_survives_movement_death_and_snapshot_restore() {
        fn setup() -> World {
            let mut world = team_world(vec![(11, 5)]);
            world.team_data.task_forces[0] = vec![(3, 0)];
            let mut data = team_mission();
            data.triggers[0].actions[0].kind = 7;
            data.team_definitions[0].waypoint = Some(0);
            data.waypoints.push((0, (4, 4)));
            world.attach_mission(data).unwrap();
            world
        }
        let mut world = setup();
        world.run_mission();
        let members = world.teams[0].members.clone();
        world.teams.clear();
        for unit in &mut world.units { unit.team = None; }
        let at = world.map.index(4, 4).unwrap();
        let leaving = members[2];
        world.order(crate::Order::Move { unit: leaving, x: 7, y: 4 });
        for _ in 0..100 { world.tick(); }
        assert_eq!(world.unit(leaving).unwrap().cell(), (7, 4));
        assert_eq!(world.claims[at], members[1] + 1, "leaving the head retains the next vehicle's claim");
        let snapshot = world.snapshot();
        let mut restored = setup();
        restored.restore(&snapshot).unwrap();
        assert_eq!(world.state_hash(), restored.state_hash());
        for candidate in [&mut world, &mut restored] {
            candidate.kill_unit_for_tests(members[1]);
            candidate.tick();
            assert_eq!(candidate.claims[at], members[0] + 1);
            assert!(candidate.stacked_claims.is_empty(), "a lone surviving claim needs no overflow list");
        }
        assert_eq!(world.state_hash(), restored.state_hash());
    }

    #[test]
    fn unsupported_reinforcement_keeps_a_specific_gap_and_creates_no_partial_team() {
        for property in ["Droppod"] {
            let mut world = team_world(vec![(11, 5)]);
            world.unit_types[0].passengers = 5;
            let mut data = team_mission();
            data.triggers[0].actions[0].kind = 7;
            data.team_definitions[0].waypoint = Some(0);
            data.team_definitions[0].properties.push((property.into(), "yes".into()));
            data.waypoints.push((0, (4, 4)));
            world.attach_mission(data).unwrap();
            world.run_mission();
            assert!(world.units.is_empty() && world.teams.is_empty());
            assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::ReinforcementUnverified { feature, .. } if feature == "DroppodCargoClass")));
        }
    }

    fn full_transport_world(only_transports: bool, carriers: u32, script: Vec<(i32, i32)>) -> World {
        let mut world = team_world(script);
        world.unit_types[0].passengers = 1;
        world.unit_types[0].size_limit = 1;
        world.unit_types.push(crate::UnitType { size: 9, stats: crate::UnitStats { infantry: true, speed_type: crate::SpeedType::Foot, kind: 2, strength: 100, speed: 6, ..crate::UnitStats::default() }, ..crate::UnitType::default() });
        world.team_data.task_forces[0] = vec![(3, 1), (carriers, 0)];
        world.team_data.team_types[0].reinforce = true;
        let mut data = team_mission();
        data.yuri = true;
        data.triggers[0].actions[0].kind = 7;
        data.triggers.push(trigger("packed-death", 48, 0, Vec::new()));
        data.tags.push(TagDef { id: "packed-tag".into(), trigger: "packed-death".into(), persistence: 1, ..TagDef::default() });
        data.team_definitions[0].tag = Some("packed-tag".into());
        data.team_definitions[0].waypoint = Some(0);
        data.team_definitions[0].properties.extend([("Full".into(), "yes".into()), ("OnTransOnly".into(), if only_transports { "yes" } else { "no" }.into()), ("VeteranLevel".into(), "3".into())]);
        data.waypoints.push((0, (4, 4)));
        data.house_edges.push((2, 2));
        world.attach_mission(data).unwrap();
        world
    }

    #[test]
    fn full_packs_every_ordinary_member_into_the_last_carrier_without_boarding_limits() {
        for only_transports in [false, true] {
            let mut world = full_transport_world(only_transports, 2, vec![(5, 100)]);
            let rng = (world.random, world.idle_random);
            world.run_mission();
            assert_eq!((world.random, world.idle_random), rng);
            let team = &world.teams[0];
            let id = team.id;
            let tag = team.scenario_tag.unwrap();
            assert_eq!(team.members, [id + 5, id + 4, id + 3, id + 2, id + 1]);
            assert_eq!(world.units.len(), 2, "only the returned carrier chain is Unlimbo'd");
            let head = world.unit(id + 5).unwrap();
            assert_eq!(head.cargo.iter().map(|unit| unit.id).collect::<Vec<_>>(), [id + 1, id + 2, id + 3]);
            assert!(world.unit(id + 4).unwrap().cargo.is_empty());
            assert_eq!(head.cargo.len(), 3, "capacity1 and size_limit1 must not reject Full's three size9 passengers");
            assert_eq!((head.cell(), head.facing), ((4, 4), 128));
            assert!(head.cargo.iter().all(|unit| (unit.x, unit.y, unit.facing, unit.veterancy, unit.team, unit.initiated) == (0, 0, 0, 2.0, Some(id), true)));
            let state = world.mission_state.as_ref().unwrap();
            assert_eq!(state.tag_refs(tag), if only_transports { 2 } else { 5 });
            for passenger in &head.cargo {
                assert_eq!(state.object_tag(crate::combat::Aim::Unit(passenger.id)), (!only_transports).then_some(tag));
                assert!(world.unit(passenger.id).is_none());
                assert!(world.registered_unit(passenger.id).is_some());
            }
            assert_eq!(world.spots[world.map.index(4, 4).unwrap()], 0, "limbo infantry cannot occupy the seed");
            let extra = world.add_infantry((10, 10), 2, 0, world.unit_types[1].stats, 2).unwrap();
            assert_eq!(world.recruit_team(id).unwrap(), 0, "limbo cargo already satisfies the TaskForce's infantry slots");
            assert_eq!(world.unit(extra).unwrap().team, None);
            world.run_teams();
            assert_eq!(world.teams[0].members.len(), 5, "off-map cargo stays a Team member");
        }
    }

    #[test]
    fn full_unloading_pops_the_native_head_and_replays_while_cargo_remains_off_map() {
        let mut world = full_transport_world(false, 1, vec![(8, 0), (5, 100)]);
        world.run_mission();
        let team = world.teams[0].id;
        let carrier = team + 4;
        let mut seen = Vec::new();
        for _ in 0..100 {
            world.tick();
            for unit in &world.units {
                if unit.id != carrier && !seen.contains(&unit.id) { seen.push(unit.id); }
            }
            if !seen.is_empty() { break; }
        }
        assert_eq!(seen, [team + 3]);
        assert_eq!(world.unit(carrier).unwrap().cargo.len(), 2);
        let saved = world.snapshot();
        let mut restored = full_transport_world(false, 1, vec![(8, 0), (5, 100)]);
        restored.restore(&saved).unwrap();
        for _ in 0..120 {
            world.tick(); restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
            for unit in &world.units {
                if unit.id != carrier && !seen.contains(&unit.id) { seen.push(unit.id); }
            }
        }
        assert_eq!(seen, [team + 3, team + 2, team + 1]);
        assert!(world.unit(carrier).unwrap().cargo.is_empty());
        assert_eq!(world.teams[0].members.len(), 4);
        assert!(world.teams[0].members.iter().all(|&id| world.unit(id).unwrap().team == Some(team)));
    }

    #[test]
    fn deleting_a_full_team_liberates_off_map_passengers_but_human_owners_keep_tags() {
        for human in [false, true] {
            let mut world = full_transport_world(false, 1, vec![(5, 0)]);
            world.houses[2].human = human;
            world.run_mission();
            let team = world.teams[0].id;
            let tag = world.teams[0].scenario_tag.unwrap();
            world.run_teams(); world.run_teams();
            assert!(world.teams.is_empty());
            for id in (team + 1)..=(team + 4) {
                assert_eq!(world.registered_unit(id).unwrap().team, None);
                assert_eq!(world.mission_state.as_ref().unwrap().object_tag(crate::combat::Aim::Unit(id)), human.then_some(tag));
            }
            assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), if human { 4 } else { 0 });
            assert_eq!(world.mission_state.as_ref().unwrap().tags[tag].destroyed, !human);
        }
    }

    #[test]
    fn full_naval_filter_is_yuri_only_and_scans_zero_amount_types_before_packing() {
        for yuri in [false, true] { for naval_amount in [0, 1] {
            let mut world = team_world(vec![(5, 100)]);
            world.unit_types[0].passengers = 1;
            world.unit_types.push(crate::UnitType { passengers: 1, naval: true, stats: crate::UnitStats { kind: 2, ..world.unit_types[0].stats }, ..crate::UnitType::default() });
            world.unit_types.push(crate::UnitType { stats: crate::UnitStats { infantry: true, kind: 3, ..world.unit_types[0].stats }, ..crate::UnitType::default() });
            world.team_data.task_forces[0] = vec![(1, 0), (naval_amount, 1), (1, 2)];
            let mut data = team_mission();
            data.yuri = yuri;
            data.triggers[0].actions[0].kind = 7;
            data.team_definitions[0].waypoint = Some(0);
            data.team_definitions[0].properties.extend([("Full".into(), "yes".into()), ("OnTransOnly".into(), "yes".into())]);
            data.triggers.push(trigger("packed-death", 48, 0, Vec::new()));
            data.tags.push(TagDef { id: "packed-tag".into(), trigger: "packed-death".into(), persistence: 1, ..TagDef::default() });
            data.team_definitions[0].tag = Some("packed-tag".into());
            data.waypoints.push((0, (4, 4)));
            world.attach_mission(data).unwrap();
            world.run_mission();
            let team = &world.teams[0];
            let ground = team.id + 1;
            let tag = team.scenario_tag.unwrap();
            assert_eq!(world.mission_state.as_ref().unwrap().object_tag(crate::combat::Aim::Unit(ground)), Some(tag), "OnTransOnly tests capacity before YR can classify the APC as cargo");
            if naval_amount == 1 {
                let naval = world.unit(team.id + 2).unwrap();
                assert_eq!(world.units.len(), if yuri { 1 } else { 2 });
                assert_eq!(naval.cargo.iter().map(|unit| unit.id).collect::<Vec<_>>(), if yuri { vec![ground, team.id + 3] } else { vec![team.id + 3] });
                assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), 2);
            } else if yuri {
                assert_eq!(world.units.len(), 2, "zero-amount Naval type still excludes the nonnaval carrier, leaving no carrier chain");
                assert!(world.units.iter().all(|unit| unit.cargo.is_empty()));
            } else {
                assert_eq!(world.units.len(), 1);
                assert_eq!(world.unit(ground).unwrap().cargo[0].id, team.id + 2);
            }
            assert_eq!(team.members.len(), 2 + naval_amount as usize);
        }}
    }

    #[test]
    fn zero_amount_air_carrier_changes_full_classification_without_creating_aircraft() {
        for air_amount in [0, 1] {
            let mut world = team_world(vec![(5, 100)]);
            world.unit_types[0].passengers = 1;
            world.unit_types.push(crate::UnitType { stats: crate::UnitStats { infantry: true, kind: 2, ..world.unit_types[0].stats }, ..crate::UnitType::default() });
            world.unit_types.push(crate::UnitType { passengers: 5, stats: crate::UnitStats { kind: 3, plane: Some(crate::aircraft::PlaneStats { flight_level: 1500, fighter: false, airport_bound: false, landable: false, ammo: 0, slowdown: 500, pitch_speed: 250 }), ..world.unit_types[0].stats }, ..crate::UnitType::default() });
            world.team_data.task_forces[0] = vec![(1, 0), (air_amount, 2), (1, 1)];
            let mut data = team_mission();
            data.triggers[0].actions[0].kind = 7;
            data.team_definitions[0].waypoint = Some(0);
            data.team_definitions[0].properties.push(("Full".into(), "yes".into()));
            data.waypoints.push((0, (4, 4)));
            world.attach_mission(data).unwrap();
            world.run_mission();
            if air_amount == 0 {
                assert_eq!(world.units.len(), 2);
                assert!(world.units.iter().all(|unit| unit.plane.is_none() && unit.cargo.is_empty()));
            } else {
                assert!(world.units.is_empty() && world.teams.is_empty());
                assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::ReinforcementUnverified { feature, .. } if feature == "AirLocomotor")));
            }
        }
    }

    #[test]
    fn change_house_hands_over_idle_base_buildings_without_resetting_combat_state_or_alliances() {
        fn setup(yuri: bool, existing_primary: bool) -> (World, Vec<u32>) {
            let mut world = field();
            world.map.shroud_on = true;
            world.map.reveal_all(1, false);
            world.building_types = vec![
                crate::BuildingType { strength: 100, factory: Some(crate::Factory::Building), construction_yard: true, size: (1, 1), ..crate::BuildingType::default() },
                crate::BuildingType { strength: 100, power: 200, size: (1, 1), ..crate::BuildingType::default() },
                crate::BuildingType { strength: 100, radar: true, powered: true, power: -100, size: (1, 1), ..crate::BuildingType::default() },
                crate::BuildingType { strength: 100, spy_sat: true, powered: true, power: -300, size: (1, 1), cash: (90, 5, 23), ..crate::BuildingType::default() },
            ];
            if existing_primary { world.add_building(1, 0, (9, 9)); }
            let mut ids = Vec::new();
            for kind in 0..4 {
                world.add_building(0, kind, (2 + kind as i32 * 2, 3));
                let building = world.buildings.last_mut().unwrap();
                building.buildup = None;
                building.rally = Some((12, 12));
                building.repairing = true;
                building.ammo = 7;
                building.health = 83;
                building.arms = crate::combat::Arms::facing(83);
                building.arms.target = Some(crate::combat::Aim::Cell(10, 10));
                building.arms.attacking = true;
                building.arms.fired_at = 47;
                building.arms.firing = 3;
                building.cash_at = 5000;
                ids.push(building.id);
            }
            let mut data = mission(vec![trigger("handover", 8, 0, vec![ActionDef::with_parameter(14, "1")])]);
            data.yuri = yuri;
            data.passive_houses = vec![0];
            for &id in &ids { data.objects.push(ObjectBinding { section: "Structures".into(), id: id.to_string(), object: crate::combat::Aim::Building(id), tag: Some(data.tags[0].id.clone()), initial_mission: "Guard".into(), group: -1 }); }
            world.attach_mission(data).unwrap();
            (world, ids)
        }
        for yuri in [false, true] { for existing_primary in [false, true] {
            let (mut world, ids) = setup(yuri, existing_primary);
            let alliances: Vec<_> = world.houses.iter().map(|house| house.allies).collect();
            world.run_mission();
            for (kind, &id) in ids.iter().enumerate() {
                let building = world.buildings.iter().find(|building| building.id == id).unwrap();
                assert_eq!(building.owner, 1);
                assert!(building.captured && !building.repairing);
                assert_eq!(building.primary, kind == 0 && !existing_primary);
                assert_eq!(building.health, 83);
                assert_eq!(building.ammo, 7);
                assert_eq!(building.arms.turret_facing(), 83);
                assert_eq!((building.arms.fired_at, building.arms.firing), (47, 3));
                assert!(building.arms.target.is_none() && !building.arms.attacking);
                if kind == 0 { assert!(building.rally.is_none()); }
            }
            assert_eq!(world.power(0), (0, 0));
            assert_eq!(world.power(1), (166, 400));
            assert!(!world.radar_available(1), "transferred power drain still gates radar");
            world.look();
            assert_ne!(world.map.satellite & crate::house_bit(1), 0, "SpySat has no native power gate");
            assert_eq!(world.houses[1].credits, 90);
            assert_eq!(world.buildings.iter().find(|building| building.id == ids[3]).unwrap().cash_at, 23);
            assert_eq!(world.houses.iter().map(|house| house.allies).collect::<Vec<_>>(), alliances);
            assert!(world.mission_state.as_ref().unwrap().destructions.is_empty(), "ownership notification is not final death");
            assert!(world.mission_state.as_ref().unwrap().tags.iter().all(|tag| !tag.executing));
            let save = world.snapshot();
            let (mut restored, _) = setup(yuri, existing_primary);
            restored.restore(&save).unwrap();
            for _ in 0..30 { world.tick(); restored.tick(); }
            assert_eq!(world.state_hash(), restored.state_hash());
        }}
    }

    #[test]
    fn change_house_transfers_an_initial_empty_refinery_without_moving_its_houses_cash_wallet() {
        for initial_structure in [false, true] {
            let mut world = field();
            world.houses[0].credits = 5000;
            world.houses[1].credits = 27;
            world.building_types = vec![crate::BuildingType { strength: 100, size: (1, 1), storage: 1000, refinery: true, ..crate::BuildingType::default() }];
            world.add_building(0, 0, (4, 4));
            let id = world.buildings[0].id;
            world.buildings[0].buildup = None;
            let mut data = mission(vec![trigger("handover", 8, 0, vec![ActionDef::with_parameter(14, "1")])]);
            data.objects.push(ObjectBinding { section: if initial_structure { "Structures" } else { "Converted" }.into(), id: "refinery".into(), object: crate::combat::Aim::Building(id), tag: Some(data.tags[0].id.clone()), initial_mission: "Guard".into(), group: -1 });
            world.attach_mission(data).unwrap();
            world.run_mission();
            assert_eq!(world.buildings[0].owner, u8::from(initial_structure));
            assert_eq!(world.houses[0].credits, 5000);
            assert_eq!(world.houses[1].credits, 27);
            let unknown = world.mission_state.as_ref().unwrap().diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::ChangeHouseUnverified { feature, .. } if feature == "StoredResourceBuildingOwnership"));
            assert_eq!(unknown, !initial_structure);
        }
    }

    #[test]
    fn change_house_leaves_active_factory_and_occupied_building_state_intact() {
        for occupied in [false, true] {
            let mut world = field();
            world.building_types = vec![crate::BuildingType { strength: 100, size: (1, 1), factory: Some(crate::Factory::Infantry), ..crate::BuildingType::default() }];
            world.add_building(0, 0, (4, 4));
            let id = world.buildings[0].id;
            world.buildings[0].buildup = None;
            let passenger = world.make_unit((0, 0), 0, crate::UnitStats::default(), 0);
            if occupied { world.buildings[0].occupants.push(passenger); }
            else { world.houses[0].infantry = Some(crate::Production { building: 0, stage: 3, rate: 4, next: 7, balance: 100, suspended: false, stalled: false }); }
            let mut data = mission(vec![trigger("handover", 8, 0, vec![ActionDef::with_parameter(14, "1")])]);
            data.objects.push(ObjectBinding { section: "Structures".into(), id: "idle-bound".into(), object: crate::combat::Aim::Building(id), tag: Some(data.tags[0].id.clone()), initial_mission: "Guard".into(), group: -1 });
            world.attach_mission(data).unwrap();
            let before = bincode::serialize(&world.buildings[0]).unwrap();
            let wallet = world.houses[0].credits;
            world.run_mission();
            assert_eq!(bincode::serialize(&world.buildings[0]).unwrap(), before, "a bounded ownership branch must not half-mutate the object");
            assert_eq!(world.houses[0].credits, wallet);
            assert_eq!(world.houses[0].infantry.is_some(), !occupied);
            let feature = if occupied { "OccupiedBuildingOwnership" } else { "ActiveBuildingFactoryOwnership" };
            assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::ChangeHouseUnverified { feature: actual, .. } if actual == feature)));
        }
    }

    #[test]
    fn ownership_notification_rejects_same_tag_reentry_and_does_not_suppress_later_death() {
        let mut world = field();
        let id = world.add_unit((3, 3), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 0);
        let mut death = trigger("killed", 7, 0, vec![ActionDef::with_parameter(19, "Death")]);
        death.linked = Some("changed".into());
        let changed = trigger("changed", 48, 0, vec![ActionDef::with_parameter(14, "1")]);
        let mut data = mission(vec![death, changed]);
        data.tags.truncate(1);
        data.tags[0].persistence = 2;
        let mut binding = placed(id, -1);
        binding.tag = Some(data.tags[0].id.clone());
        data.objects.push(binding);
        world.attach_mission(data).unwrap();
        let data = world.mission_data.take().unwrap();
        let mut state = world.mission_state.take().unwrap();
        world.raise_owner_change_events(&data, &mut state, crate::combat::Aim::Unit(id));
        assert_eq!(state.triggers.iter().find(|trigger| trigger.id == "changed").unwrap().fire_count, 2, "only the outer48 then29 notifications fire: remembered48 accepts29 too; Action14's nested notifications cannot reenter");
        assert_eq!(world.unit(id).unwrap().owner, 1);
        assert!(state.destructions.is_empty());
        world.mission_data = Some(data);
        world.mission_state = Some(state);
        world.register_mission_destruction(crate::combat::Aim::Unit(id), true);
        assert_eq!(fired(&world, "killed"), 2, "the final7 then48 sources both see the now-remembered7");
        assert_eq!(fired(&world, "changed"), 4, "remembered48 remains eligible on both final death sources");
    }

    #[test]
    fn change_house_uses_runtime_trigger_identity_and_preserves_the_native_edition_differences() {
        fn setup(yuri: bool, native_human: bool) -> World {
            let mut world = tagged_reinforcements(2, 48, 2);
            world.houses.extend([House { human: true, country: 2, ..House::default() }, House { human: true, country: 2, ..House::default() }]);
            let mut data = world.mission_data.take().unwrap();
            data.yuri = yuri;
            data.human_player_houses = Some(vec![1]);
            data.country_names.push("New Country".into());
            data.houses.extend([
                HouseBinding { name: "Controlled Other".into(), country: "New Country".into(), house: 3 },
                HouseBinding { name: "Same Country Later".into(), country: "New Country".into(), house: 4 },
            ]);
            let duplicate = data.triggers[0].actions[0].clone();
            data.triggers[0].actions.push(duplicate);
            data.triggers.iter_mut().find(|trigger| trigger.id == "death").unwrap().actions = vec![ActionDef::with_parameter(14, if native_human { "1" } else { "2" })];
            world.attach_mission(data).unwrap();
            world
        }
        for yuri in [false, true] { for native_human in [false, true] {
            let mut world = setup(yuri, native_human);
            world.run_mission();
            let first_team = world.teams[0].id;
            let second_team = world.teams[1].id;
            let members = world.teams[0].members.clone();
            let others = world.teams[1].members.clone();
            let tag = world.teams[0].scenario_tag.unwrap();
            for &id in &members {
                world.assign_unit_mission(id, UnitMission::Hunt);
                let unit = world.units.iter_mut().find(|unit| unit.id == id).unwrap();
                unit.arms.target = Some(crate::combat::Aim::Cell(10, 10));
                unit.dest = Some((12, 12));
                unit.work.archive = Some((13, 13));
            }
            let data = world.mission_data.take().unwrap();
            let mut state = world.mission_state.take().unwrap();
            world.raise_tag_mission_event(&data, &mut state, tag, MissionEventContext { source: None, kind: 48, object: None, cell: None });
            world.mission_data = Some(data);
            world.mission_state = Some(state);
            let target = if native_human { 1 } else { 3 };
            for &id in &members {
                let unit = world.unit(id).unwrap();
                assert_eq!(unit.owner, target, "Country index2 selects first matching House3, not House2 or later House4");
                assert!(unit.arms.target.is_none() && unit.dest.is_none());
                assert_eq!(unit.team, (!(yuri && native_human)).then_some(first_team));
                assert_eq!(unit.work.archive, (!yuri).then_some((13, 13)));
                assert_eq!(world.unit_mission(id), Some(if yuri { UnitMission::Guard } else { UnitMission::Hunt }));
                assert_eq!(world.mission_state.as_ref().unwrap().object_tag(crate::combat::Aim::Unit(id)), Some(tag));
            }
            assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), 2);
            for &id in &others {
                assert_eq!(world.unit(id).unwrap().owner, 2, "another Team's same authored Trigger ID is a different runtime Trigger");
                assert_eq!(world.unit(id).unwrap().team, Some(second_team));
            }
            assert_eq!(world.teams.iter().any(|team| team.id == first_team), !(yuri && native_human));
            let save = world.snapshot();
            let mut restored = setup(yuri, native_human);
            restored.restore(&save).unwrap();
            for _ in 0..20 { world.tick(); restored.tick(); }
            assert_eq!(world.state_hash(), restored.state_hash());
        }}
    }

    #[test]
    fn change_house_handles_the_exact_yr_runtime_house_constant_and_bounds_complex_objects() {
        for (value, runtime_house, expected) in [(8997, Some(1), 1), (8997, None, 0), (9001, Some(1), 0)] {
            let mut world = field();
            let id = world.add_unit((4, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 0);
            let loaded = world.add_unit((5, 5), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 0);
            let mut passenger = world.make_unit((0, 0), 0, crate::UnitStats::default(), 0);
            passenger.x = 0; passenger.y = 0;
            world.units.iter_mut().find(|unit| unit.id == loaded).unwrap().cargo.push(passenger);
            let mut data = mission(vec![trigger("change", 8, 0, vec![ActionDef::with_parameter(14, value.to_string())])]);
            data.yuri = true;
            for unit in [id, loaded] {
                let mut binding = placed(unit, -1);
                binding.tag = Some(data.tags[0].id.clone());
                data.objects.push(binding);
            }
            world.attach_mission(data).unwrap();
            world.mission_state.as_mut().unwrap().triggers[0].house = runtime_house;
            world.run_mission();
            assert_eq!(world.unit(id).unwrap().owner, expected);
            assert_eq!(world.unit(loaded).unwrap().owner, 0);
            if expected == 1 {
                assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::ChangeHouseUnverified { feature, .. } if feature == "NestedOrSpawnerOwnership")));
            }
        }
    }

    #[test]
    fn a_retreating_delivery_uninitializes_cargo_then_remembers_leaves_map_before_deleting_its_team() {
        fn setup() -> World {
            let mut world = tagged_reinforcements(1, 48, 2);
            world.unit_types[0].stats.infantry = true;
            world.unit_types[0].stats.speed_type = crate::SpeedType::Foot;
            let mut data = world.mission_data.take().unwrap();
            let mut exit = trigger("exit", 23, 0, vec![ActionDef::with_parameter(19, "ExitAfterDelay")]);
            exit.country = "Enemy Country".into();
            exit.events[0].argument = "map-team".into();
            exit.events.push(EventDef { kind: 13, parameter: 2, ..EventDef::default() });
            data.triggers.push(exit);
            data.tags.push(TagDef { id: "exit-tag".into(), trigger: "exit".into(), persistence: 0, ..TagDef::default() });
            world.unit_types.push(crate::UnitType { stats: crate::UnitStats {
                kind: 2, strength: 400, speed: 15, rot: 2,
                plane: Some(crate::aircraft::PlaneStats { flight_level: 1500, fighter: false, airport_bound: false, landable: false, ammo: 100, slowdown: 500, pitch_speed: 250 }),
                ..crate::UnitStats::default()
            }, ..crate::UnitType::default() });
            world.attach_mission(data).unwrap();
            world
        }
        let mut world = setup();
        world.run_mission();
        let members = world.teams[0].members.clone();
        let tag = world.teams[0].scenario_tag.unwrap();
        let mut cargo = Vec::new();
        for &id in members.iter().rev() {
            let at = world.units.iter().position(|unit| unit.id == id).unwrap();
            cargo.push(world.lift(at));
        }
        let plane = world.create_campaign_paradrop(1, 2, (4, 4), (4, 4), cargo);
        let at = world.units.iter().position(|unit| unit.id == plane).unwrap();
        world.units[at].x = -256;
        world.units[at].plane.as_mut().unwrap().mission = crate::aircraft::Mission::Retreat;
        world.fly_planes();
        assert!(world.teams.is_empty());
        assert!(world.unit(plane).unwrap().cargo.is_empty());
        assert!(members.iter().all(|&id| world.registered_unit(id).is_none()));
        assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), 0);
        assert_eq!(fired(&world, "death"), 0, "UnInit is not RegisterDestruction48");
        assert_eq!(fired(&world, "exit"), 0, "AND timer still waits");
        let exit = world.mission_state.as_ref().unwrap().triggers.iter().find(|trigger| trigger.id == "exit").unwrap();
        assert!(exit.occurred[0], "nonnull House sets repeat even for persistence0");
        let save = world.snapshot();
        let mut restored = setup();
        restored.restore(&save).unwrap();
        for candidate in [&mut world, &mut restored] {
            candidate.tick = 30;
            candidate.run_mission();
            assert_eq!(fired(candidate, "exit"), 1, "the deleted Team must no longer be required after its event was remembered");
        }
        assert_eq!(world.state_hash(), restored.state_hash());
    }

    #[test]
    fn leaves_map_broadcast_order_and_null_house_repeat_follow_original_registration() {
        for reverse in [false, true] {
            let mut world = field();
            world.attach_mission(mission(vec![trigger("first", 23, 0, vec![]), trigger("second", 23, 0, vec![])])).unwrap();
            world.take_mission_outputs();
            world.register_mission_team_exit(reverse);
            let names: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::TriggerFired { trigger_id, .. } = output { Some(trigger_id) } else { None }).collect();
            assert_eq!(names, if reverse { vec!["second", "first"] } else { vec!["first", "second"] });
        }
        for persistent in [false, true] {
            let mut world = field();
            let mut exit = trigger("exit", 23, 0, vec![]);
            exit.events.push(EventDef { kind: 13, parameter: 1, ..EventDef::default() });
            let mut data = mission(vec![exit]);
            data.tags[0].persistence = if persistent { 2 } else { 0 };
            world.attach_mission(data).unwrap();
            world.register_mission_team_exit(false);
            assert_eq!(world.mission_state.as_ref().unwrap().triggers[0].occurred[0], persistent);
            world.tick = 15;
            world.run_mission();
            assert_eq!(fired(&world, "exit"), u64::from(persistent));
        }
    }

    #[test]
    fn droppod_uses_an_independent_plane_and_keeps_passenger_team_tags_through_save_and_landing() {
        fn setup() -> World {
            let mut world = team_world(vec![(5, 100)]);
            world.houses[2].human = true;
            world.unit_types[0].stats.infantry = true;
            world.unit_types[0].stats.speed_type = crate::SpeedType::Foot;
            world.unit_types.push(crate::UnitType { stats: crate::UnitStats {
                kind: 2, strength: 400, speed: 15, rot: 2,
                plane: Some(crate::aircraft::PlaneStats { flight_level: 400, fighter: false, airport_bound: false, landable: false, ammo: 100, slowdown: 500, pitch_speed: 250 }),
                ..crate::UnitStats::default()
            }, ..crate::UnitType::default() });
            world.super_rules.plane = Some(1);
            world.super_rules.radius = 1024;
            world.super_rules.fall_rate = 10;
            world.team_data.task_forces[0] = vec![(3, 0)];
            let mut data = team_mission();
            data.triggers[0].actions[0].kind = 7;
            data.team_definitions[0].waypoint = Some(0);
            data.team_definitions[0].transport_waypoint = Some(1);
            data.team_definitions[0].properties.extend([("Droppod".into(), "yes".into()), ("UseTransportOrigin".into(), "yes".into())]);
            data.team_definitions[0].tag = Some("passenger-tag".into());
            data.waypoints.extend([(0, (8, 8)), (1, (8, 8))]);
            data.triggers.push(trigger("passenger-death", 48, 0, vec![]));
            data.tags.push(TagDef { id: "passenger-tag".into(), trigger: "passenger-death".into(), persistence: 1, ..TagDef::default() });
            world.attach_mission(data).unwrap();
            world
        }
        let mut world = setup();
        world.run_mission();
        let team = world.teams[0].id;
        let members = world.teams[0].members.clone();
        let tag = world.teams[0].scenario_tag.unwrap();
        assert_eq!(world.units.len(), 1, "passengers remain limbo until a real plane drop");
        let plane = world.units[0].id;
        assert!(world.units[0].team.is_none());
        assert!(world.mission_state.as_ref().unwrap().object_tag(crate::combat::Aim::Unit(plane)).is_none());
        assert_eq!(world.units[0].cargo.iter().rev().map(|unit| unit.id).collect::<Vec<_>>(), members);
        assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(tag), 3);
        for _ in 0..6 { world.tick(); }
        assert_eq!(world.units.iter().filter(|unit| unit.falling.is_some()).count(), 1);
        assert_eq!(world.unit(plane).unwrap().cargo.len(), 2);
        let save = world.snapshot();
        let mut restored = setup();
        restored.restore(&save).unwrap();
        for candidate in [&mut world, &mut restored] {
            for _ in 0..160 { candidate.tick(); }
            for &id in &members {
                let unit = candidate.unit(id).expect("native head chain must all be released");
                assert!(unit.falling.is_none());
                assert_eq!(unit.team, Some(team));
                assert_eq!(candidate.mission_state.as_ref().unwrap().object_tag(crate::combat::Aim::Unit(id)), Some(tag));
            }
            assert_eq!(fired(candidate, "passenger-death"), 0);
        }
        assert_eq!(world.state_hash(), restored.state_hash());
    }

    #[test]
    fn full_loaded_open_topped_carrier_is_explicitly_bounded_but_empty_carrier_is_supported() {
        for has_passengers in [false, true] {
            let mut world = full_transport_world(false, 1, vec![(5, 100)]);
            let mut data = world.mission_data.take().unwrap();
            data.unit_open_topped.push(0);
            if !has_passengers { world.team_data.task_forces[0] = vec![(1, 0)]; }
            world.attach_mission(data).unwrap();
            world.run_mission();
            if has_passengers {
                assert!(world.units.is_empty() && world.teams.is_empty());
                assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|diagnostic| matches!(&diagnostic.kind, DiagnosticKind::ReinforcementUnverified { feature, .. } if feature == "OpenToppedPassengerBehavior")));
            } else {
                assert_eq!(world.units.len(), 1);
                assert!(world.units[0].cargo.is_empty());
            }
        }
    }

    #[test]
    fn script_local_set_clear_share_change_only_timer_notifications_and_complete_invalid_ids() {
        fn setup(yuri: bool) -> World {
            let mut world = full_transport_world(false, 1, vec![(39, 3), (39, 3), (40, 3), (39, 99), (40, 99), (5, 100)]);
            let mut data = world.mission_data.take().unwrap();
            data.yuri = yuri;
            data.local_variable_limit = if yuri { 100 } else { 50 };
            let mut observer = trigger("observer", 13, 1, Vec::new());
            observer.enabled = false;
            observer.events.push(EventDef { kind: 36, parameter: 3, ..EventDef::default() });
            data.triggers.push(observer);
            data.tags.push(TagDef { id: "observer-tag".into(), trigger: "observer".into(), persistence: 2, ..TagDef::default() });
            world.attach_mission(data).unwrap();
            world
        }
        for yuri in [false, true] {
            let mut world = setup(yuri);
            world.tick = 5;
            world.run_mission();
            world.run_teams();
            let observer = world.mission_state.as_ref().unwrap().triggers.iter().position(|trigger| trigger.id == "observer").unwrap();
            let state = world.mission_state.as_ref().unwrap();
            assert!(state.local(3));
            assert_eq!((state.triggers[observer].timer.start_tick, state.triggers[observer].reset_generation), (5, 2));
            assert!(!state.triggers[observer].enabled);
            world.tick = 6; world.run_teams();
            assert_eq!(world.mission_state.as_ref().unwrap().triggers[observer].reset_generation, 2, "same assignment must not reset the timer");
            let saved = world.snapshot();
            let mut restored = setup(yuri);
            restored.restore(&saved).unwrap();
            for tick in 7..=10 {
                world.tick = tick; restored.tick = tick;
                world.run_teams(); restored.run_teams();
                assert_eq!(world.snapshot(), restored.snapshot());
                if tick == 8 { assert_eq!(world.mission_state.as_ref().unwrap().local(99), yuri); }
            }
            let state = world.mission_state.as_ref().unwrap();
            assert!(!state.local(3) && !state.local(99));
            assert_eq!((state.triggers[observer].timer.start_tick, state.triggers[observer].reset_generation), (7, 3));
            assert_eq!(world.teams[0].line, 5, "out-of-range setters must still complete the step");
            assert_eq!(state.diagnostics.iter().filter(|diagnostic| matches!(diagnostic.kind, DiagnosticKind::InvalidParameter { kind: 39 | 40, .. })).count(), if yuri { 0 } else { 2 });
        }
    }

    #[test]
    fn a_full_team_unloads_releases_its_carrier_then_sets_the_campaign_landing_flag() {
        let mut world = full_transport_world(false, 1, vec![(8, 2), (39, 3), (5, 100)]);
        let mut data = world.mission_data.take().unwrap();
        data.triggers.push(trigger("landed", 36, 3, vec![ActionDef::with_parameter(11, "beach-cleared")]));
        data.tags.push(TagDef { id: "landing-tag".into(), trigger: "landed".into(), persistence: 0, ..TagDef::default() });
        world.attach_mission(data).unwrap();
        world.run_mission();
        let team = world.teams[0].id;
        let carrier = team + 4;
        for _ in 0..200 { world.tick(); if fired(&world, "landed") > 0 { break; } }
        assert_eq!(fired(&world, "landed"), 1);
        assert!(world.mission_state.as_ref().unwrap().local(3));
        assert!(world.unit(carrier).unwrap().cargo.is_empty());
        assert_eq!(world.unit(carrier).unwrap().team, None);
        assert_eq!(world.teams[0].members.len(), 3);
        assert!(world.teams[0].members.iter().all(|&id| world.unit(id).unwrap().team == Some(team)));
        assert!(world.take_mission_outputs().contains(&MissionOutput::Text { label: "beach-cleared".into() }));
    }


    #[test]
    fn empty_reinforcement_script_gets_shared_guard_overlay_without_changing_static_rules() {
        let mut world = team_world(Vec::new());
        let mut data = team_mission();
        data.triggers[0].actions[0].kind = 7;
        data.team_definitions[0].waypoint = Some(0);
        data.waypoints.push((0, (4, 4)));
        world.attach_mission(data).unwrap();
        let hash = world.mission_team_rules_hash();
        world.run_mission();
        assert_eq!(world.mission_team_rules_hash(), hash);
        assert!(world.team_data.scripts[0].is_empty());
        assert_eq!(world.mission_state.as_ref().unwrap().default_script_overrides, vec![0]);
        world.run_teams();
        assert_eq!(world.teams[0].line, 0);
    }

    #[test]
    fn full_without_transport_fills_type_zero_ore_rather_than_changing_health() {
        let mut world = team_world(vec![(11, 5)]);
        world.unit_types[0].stats.storage = 20;
        let mut data = team_mission();
        data.triggers[0].actions[0].kind = 7;
        data.team_definitions[0].waypoint = Some(0);
        data.team_definitions[0].properties.push(("Full".into(), "yes".into()));
        data.waypoints.push((0, (4, 4)));
        world.attach_mission(data).unwrap();
        world.run_mission();
        let unit = world.unit(world.teams[0].members[0]).unwrap();
        assert_eq!(unit.work.cargo, [20, 0, 0, 0]);
        assert_eq!(unit.health, unit.stats.strength);
    }

    #[test]
    fn full_size_coordinates_are_separate_from_usable_area_and_cell_storage() {
        let mut map = Map::new(3, 3);
        map.set_playfield(8, 8, [2, 2, 2, 2]);
        assert!(!map.coordinates_legal(4, 4), "the upper edge is strict");
        assert!(map.coordinates_legal(5, 4));
        assert!(map.coordinates_legal(12, 12), "the lower edge is inclusive");
        assert!(!map.coordinates_legal(13, 12));
        assert!(!map.coordinates_legal(12, 4), "the side edges are strict");
        assert!(!map.in_playfield(5, 4), "legal original coordinates need not exist in this storage rectangle");
    }

    fn armed_world(script: Vec<(i32, i32)>) -> World {
        let mut world = team_world(script);
        world.weapons.push(crate::combat::Weapon {
            damage: 1,
            range: 2 * crate::LEPTONS_PER_CELL,
            rof: 100,
            ..crate::combat::Weapon::default()
        });
        world.projectiles.push(crate::combat::Projectile::default());
        world.warheads.push(crate::combat::Warhead::default());
        world.unit_types[0].stats.fighter = crate::combat::Fighter {
            primary: Some(crate::combat::Mount::default()),
            sight: 5,
            ..crate::combat::Fighter::default()
        };
        world
    }

    #[test]
    fn placed_sleep_prevents_acquisition_retaliation_fear_scatter_and_recruitment_until_ordered() {
        let mut world = armed_world(vec![(5, 100)]);
        let stats = crate::UnitStats {
            infantry: true,
            fighter: crate::combat::Fighter {
                fraidycat: true,
                ..world.unit_types[0].stats.fighter
            },
            ..world.unit_types[0].stats
        };
        let sleeper = world.add_infantry((4, 4), 0, 0, stats, 2).unwrap();
        let enemy = world.add_unit((5, 4), 0, world.unit_types[0].stats, 1);
        let mut data = team_mission();
        let mut binding = placed(sleeper, -1);
        binding.initial_mission = "Sleep".into();
        data.objects = vec![binding];
        world.attach_mission(data).unwrap();
        world.run_mission();
        let team = world.teams[0].id;
        assert_eq!(world.recruit_team(team).unwrap(), 0);
        world.receive_damage(
            crate::combat::Aim::Unit(sleeper),
            1,
            0,
            0,
            Some(crate::combat::Aim::Unit(enemy)),
        );
        world.run_soldiers();
        world.fight();
        let unit = world.unit(sleeper).unwrap();
        assert_eq!(unit.arms.target, None);
        assert_eq!(unit.destination(), None);
        assert!(unit.fear > 200);
        world.order(crate::Order::Stop { unit: sleeper });
        assert_eq!(world.unit_mission(sleeper), None);
        assert_eq!(world.recruit_team(team).unwrap(), 1);
    }

    #[test]
    fn explicit_area_guard_sets_a_post_and_acquires_outside_weapon_range() {
        let mut world = armed_world(Vec::new());
        let stats = world.unit_types[0].stats;
        let guard = world.add_unit((3, 4), 0, stats, 2);
        let enemy = world.add_unit((6, 4), 0, stats, 1);
        let mut data = mission(Vec::new());
        let mut binding = placed(guard, -1);
        binding.initial_mission = "Area Guard".into();
        data.objects.push(binding);
        world.attach_mission(data).unwrap();
        world.fight();
        let unit = world.unit(guard).unwrap();
        assert_eq!(unit.post, Some((3, 4)));
        assert_eq!(unit.arms.target, Some(crate::combat::Aim::Unit(enemy)));
        assert!(unit.arms.attacking);
        assert!(!stats.fighter.default_guard_area);
    }

    #[test]
    fn hunt_acquires_distant_enemy_and_replays_retarget_timers_after_restore() {
        let mut world = armed_world(Vec::new());
        let stats = world.unit_types[0].stats;
        let hunter = world.add_unit((2, 4), 0, stats, 2);
        let first = world.add_unit((10, 4), 0, stats, 1);
        let second = world.add_unit((14, 4), 0, stats, 1);
        let mut data = mission(Vec::new());
        let mut binding = placed(hunter, -1);
        binding.initial_mission = "Hunt".into();
        data.objects = vec![binding];
        world.attach_mission(data.clone()).unwrap();
        world.tick();
        assert_eq!(
            world.unit(hunter).unwrap().arms.target,
            Some(crate::combat::Aim::Unit(first))
        );
        world
            .units
            .iter_mut()
            .find(|unit| unit.id == first)
            .unwrap()
            .health = 0;
        let saved = world.snapshot();
        let mut restored = armed_world(Vec::new());
        restored.add_unit((2, 4), 0, stats, 2);
        restored.add_unit((10, 4), 0, stats, 1);
        restored.add_unit((14, 4), 0, stats, 1);
        restored.attach_mission(data).unwrap();
        restored.restore(&saved).unwrap();
        for _ in 0..40 {
            world.tick();
            restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
        }
        assert_eq!(
            world.unit(hunter).unwrap().arms.target,
            Some(crate::combat::Aim::Unit(second))
        );
        assert_eq!(world.unit_mission(hunter), Some(UnitMission::Hunt));
    }

    #[test]
    fn attack_waypoint_targets_a_building_then_advances_on_its_death() {
        let mut world = armed_world(vec![(1, 400), (5, 100)]);
        let stats = world.unit_types[0].stats;
        let member = world.add_unit((2, 4), 0, stats, 2);
        world.building_types.push(crate::BuildingType {
            strength: 100,
            size: (1, 1),
            ..crate::BuildingType::default()
        });
        world.add_building(1, 0, (10, 4));
        let target = world.buildings[0].id;
        let mut data = team_mission();
        data.waypoints = vec![(400, (10, 4))];
        world.attach_mission(data).unwrap();
        for _ in 0..5 {
            world.tick();
        }
        assert_eq!(world.teams[0].line, 0);
        assert_eq!(
            world.teams[0].target,
            Some(crate::combat::Aim::Building(target))
        );
        assert_eq!(
            world.unit(member).unwrap().arms.target,
            Some(crate::combat::Aim::Building(target))
        );
        world.buildings[0].health = 0;
        for _ in 0..3 {
            world.tick();
        }
        assert_eq!(world.teams[0].line, 1);
    }

    #[test]
    fn missing_attack_waypoint_blocks_repeatedly_and_do_mission_keeps_team_membership() {
        let mut missing = team_world(vec![(1, 400), (49, 0)]);
        missing.add_unit((4, 4), 0, missing.unit_types[0].stats, 2);
        missing.attach_mission(team_mission()).unwrap();
        for _ in 0..20 {
            missing.tick();
        }
        assert_eq!(missing.teams[0].line, 0);
        assert!(!missing.teams[0].next_mission);
        let mut world = team_world(vec![(11, 11), (49, 0)]);
        let member = world.add_unit((4, 4), 0, world.unit_types[0].stats, 2);
        world.attach_mission(team_mission()).unwrap();
        for _ in 0..20 {
            world.tick();
        }
        assert_eq!(world.teams[0].line, 0);
        assert_eq!(world.unit_mission(member), Some(UnitMission::AreaGuard));
        assert_eq!(world.unit(member).unwrap().team, Some(world.teams[0].id));
    }

    #[test]
    fn native_add_member_group_and_recruitable_flag_survive_script_end() {
        let mut world = team_world(vec![(49, 0)]);
        world.team_data.team_types[0].members_recruitable = false;
        let member = world.add_unit((4, 4), 0, world.unit_types[0].stats, 2);
        let mut data = team_mission();
        data.objects = vec![placed(member, 7)];
        // Recruiter permits the mismatch, then AddMember writes Group=8.
        world.team_data.team_types[0].recruiter = true;
        data.team_definitions[0].group = 8;
        world.attach_mission(data).unwrap();
        for _ in 0..10 {
            world.tick();
        }
        assert!(world.teams.is_empty());
        let unit = world.unit(member).unwrap();
        assert_eq!(unit.team, None);
        assert!(!unit.recruitable);
        assert_eq!(
            world.mission_state.as_ref().unwrap().unit_groups,
            vec![(member, 8)]
        );
    }

    #[test]
    fn parent_country_is_for_owner_production_while_required_and_forbidden_use_identity() {
        let mut world = team_world(Vec::new());
        world.houses[2].country = 13;
        world.houses[2].parent_country = Some(0);
        assert_eq!(world.houses[2].production_country(), 0);
        world.unit_types[0].owners = 1;
        world.unit_types[0].required_houses = Some(1 << 13);
        world.unit_types[0].forbidden_houses = Some(1 << 4);
        assert!(world.may_build(2, crate::Item::Unit(0)));
        world.unit_types[0].required_houses = Some(1);
        assert!(!world.may_build(2, crate::Item::Unit(0)));
        world.unit_types[0].required_houses = None;
        world.unit_types[0].forbidden_houses = Some(1 << 13);
        assert!(!world.may_build(2, crate::Item::Unit(0)));
        world.houses[2].parent_country = Some(31);
        assert_eq!(world.houses[2].production_country(), 31);
    }

    fn campaign_factory_world() -> (World, MissionData) {
        let mut world = team_world(vec![(5, 100)]);
        world.houses[2].credits = 10_000;
        world.houses[2].tech_level = 10;
        world.houses[2].ai = Some(crate::ai::Brain::for_campaign(
            1,
            crate::ai::CampaignState {
                iq: 1,
                ..crate::ai::CampaignState::default()
            },
        ));
        world.unit_types[0].owners = 1;
        world.unit_types[0].cost = 60;
        world.unit_types[0].tech_level = 1;
        world.building_types.push(crate::BuildingType {
            factory: Some(crate::Factory::Unit),
            owners: 1,
            size: (1, 1),
            strength: 100,
            ..crate::BuildingType::default()
        });
        world.add_map_building(2, 0, (4, 4), 256);
        let mut data = team_mission();
        data.yuri = true;
        data.country_names.push("Enemy Country".into());
        data.triggers.push(trigger(
            "begin-production",
            13,
            1,
            vec![ActionDef::with_parameter(3, "2")],
        ));
        data.tags.push(TagDef {
            id: "tag-production".into(),
            trigger: "begin-production".into(),
            ..TagDef::default()
        });
        (world, data)
    }

    #[test]
    fn campaign_iq_switches_enable_passive_houses_before_separate_ai_dispatch_gates() {
        for yuri in [false, true] { for human in [false, true] { for passive in [false, true] {
            for defeated in [false, true] { for iq in [4, 5] { for automatic in [false, true] {
                let (mut world, mut data) = campaign_factory_world();
                data.yuri = yuri;
                data.campaign_ai_rules.production = 5;
                data.triggers.clear(); data.tags.clear();
                if passive { data.passive_houses.push(2); }
                world.houses[2].human = human;
                world.houses[2].defeated = defeated;
                {
                    let control = world.houses[2].ai.as_mut().unwrap().campaign.as_mut().unwrap();
                    control.iq = iq;
                    control.auto_base_building = automatic;
                }
                world.initialize_campaign_random(123);
                world.attach_mission(data).unwrap();
                let before = world.houses[2].ai.clone().unwrap();
                let random = world.campaign_random_state().cloned();
                world.think();
                let brain = world.houses[2].ai.as_ref().unwrap();
                let control = brain.campaign.as_ref().unwrap();
                let enabled = !human && (iq >= 5 || automatic);
                assert_eq!((control.production, control.autocreate, control.auto_base_building),
                    (enabled, enabled, automatic || enabled), "YR{yuri} human{human} passive{passive} defeated{defeated} IQ{iq} auto{automatic}");
                assert_eq!(brain.triggers_active, before.triggers_active, "the automatic switch does not enable AITriggers");
                if human || passive || defeated {
                    assert_eq!((brain.expert_at, brain.team_timer, brain.awake), (before.expert_at, before.team_timer, before.awake));
                    assert_eq!((brain.build_unit, brain.build_infantry, brain.build_aircraft), (None, None, None));
                    assert!(world.houses[2].vehicles.is_none());
                    assert_eq!(world.campaign_random_state(), random.as_ref(), "blocked AI dispatch consumes no Expert or selection RNG");
                }
            }}}
        }}}
    }

    #[test]
    fn native_campaign_range_preserves_rejected_words_and_leaves_the_legacy_stream_alone() {
        let mut world = field();
        let legacy = world.random;
        world.initialize_campaign_random(0x1234_5678);
        assert_eq!(world.scenario_random_range(0, 8), 1);
        assert_eq!(world.campaign_random_state().unwrap().indices(), (4, 107),
            "the original CPU seed12345678 Range(0,8) rejects its first three words");
        let before = world.campaign_random_state().unwrap().clone();
        assert_eq!(world.scenario_random_range(5, 5), 5);
        assert_eq!(world.campaign_random_state(), Some(&before));
        assert_eq!(world.random, legacy);
        let mut no_mission = field();
        let mut expected = no_mission.random;
        crate::random_step(&mut expected);
        assert_eq!(no_mission.scenario_random_range(5, 5), 5);
        assert_eq!(no_mission.random, expected, "the old fallback still consumes one word for a single result");
        assert!(no_mission.campaign_random_state().is_none());
    }

    #[test]
    fn campaign_ai_selection_consumes_the_native_stream_before_production_and_restores_all_words() {
        let (mut world, data) = campaign_factory_world();
        world.initialize_campaign_random(0);
        world.attach_mission(data.clone()).unwrap();
        for _ in 0..14 { world.tick(); }
        assert_eq!(world.campaign_random_state().unwrap().indices(), (4, 107),
            "seed0 has one Expert.Range(1,7) then three frame8 production normalized requests");
        assert_eq!(world.houses[2].ai.as_ref().unwrap().build_unit, Some(0));
        assert!(world.houses[2].vehicles.is_none());
        let saved = world.snapshot();
        let (mut restored, _) = campaign_factory_world();
        restored.initialize_campaign_random(0xfeed_beef);
        restored.attach_mission(data).unwrap();
        restored.restore(&saved).unwrap();
        assert_eq!(world.campaign_random_state(), restored.campaign_random_state());
        for _ in 0..160 {
            world.tick(); restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
        }
        assert_eq!(world.take_mission_outputs(), restored.take_mission_outputs());
    }

    #[test]
    fn iq_one_waits_for_action_three_then_builds_the_created_teams_missing_member_and_replays() {
        let (mut world, data) = campaign_factory_world();
        world.attach_mission(data.clone()).unwrap();
        for _ in 0..14 {
            world.tick();
        }
        assert!(world.units.is_empty());
        assert!(world.houses[2].vehicles.is_none());
        assert_eq!(world.houses[2].ai.as_ref().unwrap().build_unit, Some(0),
            "frame8 selects the missing member even before Action3 enables factory production");
        assert!(
            !world.houses[2]
                .ai
                .as_ref()
                .unwrap()
                .campaign
                .as_ref()
                .unwrap()
                .production
        );
        world.tick();
        assert!(
            world.houses[2]
                .ai
                .as_ref()
                .unwrap()
                .campaign
                .as_ref()
                .unwrap()
                .production
        );
        assert!(
            world.houses[2].vehicles.is_none(),
            "selection runs on frame % 8 == 0"
        );
        world.tick();
        assert_eq!(
            world.houses[2]
                .vehicles
                .as_ref()
                .map(|production| production.building),
            Some(0)
        );
        let saved = world.snapshot();
        let (mut restored, _) = campaign_factory_world();
        restored.attach_mission(data).unwrap();
        restored.restore(&saved).unwrap();
        for _ in 0..250 {
            world.tick();
            restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
        }
        assert_eq!(world.units.len(), 1);
        assert_eq!(world.teams[0].members.len(), 1);
        assert_eq!(world.unit(world.teams[0].members[0]).unwrap().owner, 2);
        assert!(world.houses[2].credits < 10_000);
    }

    #[test]
    fn iq_one_campaign_preserves_authored_scenery_instead_of_selling_an_uneconomic_base() {
        let mut world = team_world(Vec::new());
        world.houses[2].credits = 0;
        world.houses[2].ai = Some(crate::ai::Brain::for_campaign(
            1,
            crate::ai::CampaignState {
                iq: 1,
                ..crate::ai::CampaignState::default()
            },
        ));
        world.building_types.push(crate::BuildingType {
            strength: 100,
            size: (1, 1),
            ..crate::BuildingType::default()
        });
        let scenery = world.add_map_building(2, 0, (4, 4), 256);
        world.add_map_building(1, 0, (10, 10), 256);
        world.attach_mission(mission(Vec::new())).unwrap();
        for _ in 0..1500 {
            world.tick();
        }
        let building = world
            .buildings
            .iter()
            .find(|building| building.id == scenery)
            .unwrap();
        assert_eq!(building.health, 100);
        assert!(building.buildup.is_none());
        assert!(world.houses[2].ai.as_ref().unwrap().nodes.is_empty());
        assert!(!world.houses[2].ai.as_ref().unwrap().all_hunt);
        assert_eq!(
            world.houses[2].ai.as_ref().unwrap().enemy,
            None,
            "campaign Expert skips the skirmish nearest-base fallback"
        );
    }

    #[test]
    fn ra2_selects_production_without_the_yr_eight_frame_gate_and_staggers_by_original_house_index()
    {
        let (mut world, mut data) = campaign_factory_world();
        data.yuri = false;
        world.houses[2].ai.as_mut().unwrap().original_house_index = Some(5);
        world.attach_mission(data).unwrap();
        assert_eq!(
            world.houses[2].ai.as_ref().unwrap().team_timer,
            2500 + 175 * 5
        );
        for _ in 0..14 {
            world.tick();
        }
        assert!(world.houses[2].vehicles.is_none());
        world.tick();
        assert_eq!(world.tick, 15);
        assert_eq!(
            world.houses[2]
                .vehicles
                .as_ref()
                .map(|production| production.building),
            Some(0)
        );
    }

    #[test]
    fn campaign_base_builds_only_the_authored_node_at_its_original_cell() {
        let (mut world, mut data) = campaign_factory_world();
        data.triggers.remove(0);
        data.tags.remove(0);
        world.building_types.push(crate::BuildingType {
            factory: Some(crate::Factory::Building),
            construction_yard: true,
            base_normal: true,
            owners: 1,
            size: (1, 1),
            strength: 100,
            ..crate::BuildingType::default()
        });
        world.add_map_building(2, 1, (6, 6), 256);
        world.building_types.push(crate::BuildingType {
            cost: 60,
            tech_level: 1,
            adjacent: 2,
            owners: 1,
            size: (1, 1),
            strength: 100,
            ..crate::BuildingType::default()
        });
        world.houses[2].ai.as_mut().unwrap().nodes = vec![crate::ai::Node::planned(2, (7, 6))];
        world.attach_mission(data).unwrap();
        for _ in 0..200 {
            world.tick();
        }
        let built: Vec<_> = world
            .buildings
            .iter()
            .filter(|building| building.owner == 2 && building.kind == 2)
            .map(|building| building.cell)
            .collect();
        assert_eq!(built, vec![(7, 6)]);
        assert_eq!(world.houses[2].ai.as_ref().unwrap().nodes.len(), 1);
    }

    fn campaign_ai_trigger_world() -> (World, MissionData) {
        let (mut world, mut data) = campaign_factory_world();
        data.triggers = vec![trigger(
            "begin-ai",
            13,
            1,
            vec![ActionDef::with_parameter(74, "2")],
        )];
        data.tags = vec![TagDef {
            id: "tag-ai".into(),
            trigger: "begin-ai".into(),
            ..TagDef::default()
        }];
        data.waypoints = vec![(40, (12, 12))];
        data.team_definitions[0].waypoint = Some(40);
        world.team_data.team_types[0].max = 1;
        world.team_data.team_types[0].base_defense = true;
        world.team_data.team_delays = [1; 3];
        world.team_data.triggers.push(crate::teams::Trigger {
            team1: Some(0),
            team2: None,
            owner: Some(None),
            condition: -1,
            object: None,
            operand: 0,
            operator: 0,
            weight: 1.0,
            min_weight: 1.0,
            max_weight: 1.0,
            skirmish: false,
            side: 0,
            enabled: [true; 3],
            successes: 0,
            completed: 0,
        });
        let brain = world.houses[2].ai.as_mut().unwrap();
        brain.timer_armed = true;
        brain.team_timer = 1;
        let unit = world.add_unit((4, 6), 0, world.unit_types[0].stats, 2);
        data.objects.push(placed(unit, -1));
        (world, data)
    }

    #[test]
    fn campaign_ai_trigger_starts_a_real_map_team_after_action_74_and_restores_without_mutating_rules(
    ) {
        let (mut world, data) = campaign_ai_trigger_world();
        world.attach_mission(data.clone()).unwrap();
        for _ in 0..14 {
            world.tick();
        }
        assert!(world.teams.is_empty());
        for _ in 0..6 {
            world.tick();
        }
        assert_eq!(world.teams.len(), 1);
        assert_eq!(world.teams[0].members.len(), 1);
        assert_eq!(world.teams[0].mission_type.as_deref(), Some("map-team"));
        assert_eq!(world.teams[0].scenario_waypoint, Some((12, 12)));
        assert!(!world.team_data.team_types[0].autocreate);
        assert_eq!(
            world.mission_state.as_ref().unwrap().autocreate_team_types,
            vec![0]
        );
        let saved = world.snapshot();
        let (mut restored, _) = campaign_ai_trigger_world();
        restored.attach_mission(data).unwrap();
        restored.restore(&saved).unwrap();
        for _ in 0..80 {
            world.tick();
            restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
        }
    }

    #[test]
    fn action_76_changes_context_house_ratio_without_clamping_or_enabling_legacy_autocreate() {
        let (mut world, mut data) = campaign_ai_trigger_world();
        for (id, time, value) in [("ratio-zero", 0, "0"), ("ratio-high", 2, "150")] {
            let mut definition = trigger(id, 47, time, vec![ActionDef::with_parameter(76, value)]);
            definition.country = "Enemy Country".into();
            data.triggers.push(definition);
            data.tags.push(TagDef {
                id: format!("tag-{id}"),
                trigger: id.into(),
                ..TagDef::default()
            });
        }
        world.attach_mission(data).unwrap();
        for _ in 0..25 {
            world.tick();
        }
        assert!(world.houses[2].ai.as_ref().unwrap().triggers_active);
        assert_eq!(world.houses[2].ai.as_ref().unwrap().trigger_ratio, 0);
        assert!(world.teams.is_empty());
        for _ in 0..10 {
            world.tick();
        }
        assert_eq!(world.houses[2].ai.as_ref().unwrap().trigger_ratio, 150);
        assert_eq!(world.teams.len(), 1);
    }

    #[test]
    fn action_four_creates_an_empty_team_then_recruits_without_spawning_or_using_the_trigger_house()
    {
        let mut world = team_world(vec![(5, 100)]);
        let stats = world.unit_types[0].stats;
        let unit = world.add_unit((4, 4), 0, stats, 2);
        let mut data = team_mission();
        data.tags[0].persistence = 2;
        world.attach_mission(data).unwrap();
        // Calling only the mission pass proves action 4 constructs an empty
        // team. It does not directly recruit or create the task force's units.
        world.run_mission();
        assert_eq!(world.teams.len(), 1);
        assert!(world.teams[0].members.is_empty());
        assert_eq!(world.units().len(), 1);
        let team = world.teams[0].id;
        assert_eq!(world.teams[0].house, 2);
        assert_eq!(world.recruit_team(team).unwrap(), 1);
        assert_eq!(world.unit(unit).unwrap().team, Some(team));
        // Original action 4's override counter bypasses Max=0.
        world.run_mission();
        assert_eq!(world.teams.len(), 2);
        assert_eq!(world.units().len(), 1);
    }

    #[test]
    fn campaign_recruitment_uses_original_waypoint_and_group_filters() {
        let mut world = team_world(vec![(5, 100)]);
        let stats = world.unit_types[0].stats;
        let far = world.add_unit((1, 1), 0, stats, 2);
        let near = world.add_unit((10, 10), 0, stats, 2);
        let wrong_group = world.add_unit((12, 12), 0, stats, 2);
        let mut data = team_mission();
        data.team_definitions[0].group = 7;
        data.team_definitions[0].waypoint = Some(40);
        data.waypoints = vec![(40, (12, 12))];
        data.objects = vec![placed(far, 7), placed(near, 7), placed(wrong_group, 8)];
        world.attach_mission(data).unwrap();
        world.tick();
        assert_eq!(world.teams.len(), 1);
        assert_eq!(world.teams[0].members, vec![near]);
        assert_eq!(world.teams[0].scenario_waypoint, Some((12, 12)));
        assert_eq!(world.unit(far).unwrap().team, None);
        assert_eq!(world.unit(wrong_group).unwrap().team, None);
    }

    #[test]
    fn unsupported_campaign_script_stays_on_its_line_instead_of_recording_success() {
        let mut world = team_world(vec![(999, 0), (49, 0)]);
        let stats = world.unit_types[0].stats;
        world.add_unit((4, 4), 0, stats, 2);
        world.attach_mission(team_mission()).unwrap();
        for _ in 0..30 {
            world.tick();
        }
        assert_eq!(world.teams.len(), 1);
        assert_eq!(world.teams[0].line, 0);
        let diagnostics = &world.mission_state.as_ref().unwrap().diagnostics;
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| matches!(
                    d.kind,
                    DiagnosticKind::UnsupportedScript { opcode: 999, .. }
                ))
                .count(),
            1
        );
        assert!(!world.teams[0].next_mission);
    }

    #[test]
    fn campaign_move_script_uses_waypoint_id_and_team_continues_after_restore() {
        let data = {
            let mut data = team_mission();
            data.waypoints = vec![(400, (12, 4))];
            data
        };
        let mut world = team_world(vec![(3, 400), (5, 100)]);
        let stats = world.unit_types[0].stats;
        let unit = world.add_unit((2, 4), 0x40, stats, 2);
        world.attach_mission(data.clone()).unwrap();
        for _ in 0..25 {
            world.tick();
        }
        assert_eq!(world.teams[0].target, Some(crate::combat::Aim::Cell(12, 4)));
        let saved = world.snapshot();
        let mut restored = team_world(vec![(3, 400), (5, 100)]);
        restored.add_unit((2, 4), 0x40, stats, 2);
        restored.attach_mission(data.clone()).unwrap();
        restored.restore(&saved).unwrap();
        for _ in 0..150 {
            world.tick();
            restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
        }
        assert!(world.unit(unit).unwrap().cell().0 > 2);
        let mut wrong_script = team_world(vec![(3, 401), (5, 100)]);
        wrong_script.add_unit((2, 4), 0x40, stats, 2);
        wrong_script.attach_mission(data).unwrap();
        assert!(wrong_script.restore(&saved).is_err());
    }

    #[test]
    fn invalid_task_force_members_diagnose_at_activation_without_breaking_map_attachment() {
        let mut world = team_world(vec![(5, 1)]);
        world.team_data.task_forces[0] = vec![(1, u16::MAX)];
        world.attach_mission(team_mission()).unwrap();
        world.tick();
        assert!(world.teams.is_empty());
        assert!(world
            .mission_state
            .as_ref()
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, DiagnosticKind::TeamCreationFailed { .. })));
    }

    #[test]
    fn enabling_an_enabled_trigger_resets_the_relative_timer_every_time() {
        let mut world = field();
        let mut data = mission(vec![
            trigger(
                "delayed",
                13,
                2,
                vec![ActionDef::with_parameter(11, "time up")],
            ),
            trigger(
                "reset",
                47,
                1,
                vec![ActionDef::with_parameter(53, "delayed")],
            ),
        ]);
        // reset once at tick 15; delayed was already enabled at tick 0.
        data.tags[1].persistence = 0;
        world.attach_mission(data).unwrap();
        for _ in 0..44 {
            world.tick();
        }
        assert_eq!(fired(&world, "delayed"), 0);
        assert_eq!(
            world.mission_state.as_ref().unwrap().triggers[0]
                .timer
                .start_tick,
            15
        );
        world.tick();
        assert_eq!(fired(&world, "delayed"), 1);
    }

    #[test]
    fn enable_on_the_due_tick_resets_a_later_tags_timer_before_registration() {
        let mut world = field();
        world
            .attach_mission(mission(vec![
                trigger(
                    "reset",
                    47,
                    2,
                    vec![ActionDef::with_parameter(53, "delayed")],
                ),
                trigger("delayed", 13, 2, Vec::new()),
            ]))
            .unwrap();
        for _ in 0..59 {
            world.tick();
        }
        assert_eq!(fired(&world, "delayed"), 0);
        world.tick();
        assert_eq!(fired(&world, "delayed"), 1);
    }

    #[test]
    fn relative_and_global_time_have_different_baselines() {
        let mut world = field();
        for _ in 0..60 {
            world.tick();
        }
        world
            .attach_mission(mission(vec![
                trigger("relative", 13, 2, Vec::new()),
                trigger("global", 47, 2, Vec::new()),
            ]))
            .unwrap();
        world.tick();
        assert_eq!(fired(&world, "global"), 1);
        assert_eq!(fired(&world, "relative"), 0);
        for _ in 0..29 {
            world.tick();
        }
        assert_eq!(fired(&world, "relative"), 1);
    }

    #[test]
    fn enable_is_not_recursive_but_a_later_registered_tag_can_run_in_the_same_pass() {
        for source_first in [false, true] {
            let mut world = field();
            let mut target = trigger("target", 13, 0, vec![ActionDef::with_parameter(11, "target")]);
            target.enabled = false;
            let source = trigger("source", 8, 0, vec![ActionDef::with_parameter(53, "target")]);
            world.attach_mission(mission(if source_first { vec![source, target] } else { vec![target, source] })).unwrap();
            world.tick();
            assert_eq!(fired(&world, "source"), 1);
            assert_eq!(fired(&world, "target"), u64::from(source_first));
            world.tick();
            assert_eq!(fired(&world, "target"), 1);
            assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|d| d.kind == DiagnosticKind::SchedulingUnverified));
        }
    }

    #[test]
    fn periodic_tag_registers_each_linked_trigger_after_the_preceding_actions() {
        for variable in [false, true] {
            let mut world = field();
            let mut target = trigger("target", if variable { 36 } else { 13 }, 0, Vec::new());
            target.linked = Some("source".into());
            target.enabled = variable;
            let source = trigger("source", 8, 0, vec![ActionDef::with_parameter(if variable { 56 } else { 53 }, if variable { "0" } else { "target" })]);
            let mut data = mission(vec![target, source]);
            data.tags.truncate(1); // Native constructor builds source -> target.
            world.attach_mission(data).unwrap();
            world.tick();
            assert_eq!(fired(&world, "source"), 1);
            assert_eq!(fired(&world, "target"), 1);
            let order: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::TriggerFired { trigger_id, .. } = output { Some(trigger_id) } else { None }).collect();
            assert_eq!(order, ["source", "target"]);
        }
    }

    #[test]
    fn local_set_and_clear_control_conditions_without_consuming_combat_events() {
        let mut world = field();
        world
            .attach_mission(mission(vec![
                trigger("set", 8, 0, vec![ActionDef::with_parameter(56, "5")]),
                trigger(
                    "observe",
                    36,
                    5,
                    vec![
                        ActionDef::with_parameter(11, "objective"),
                        ActionDef::with_parameter(57, "5"),
                    ],
                ),
            ]))
            .unwrap();
        world.tick();
        assert!(!world.mission_state.as_ref().unwrap().local(5));
        assert_eq!(fired(&world, "observe"), 1);
        let _ = world.take_events();
        let outputs = world.take_mission_outputs();
        assert!(outputs.contains(&MissionOutput::Text {
            label: "objective".into()
        }));
        assert!(world.take_mission_outputs().is_empty());
    }

    #[test]
    fn a_variable_change_resets_observer_timers_but_assigning_the_same_value_does_not() {
        let mut world = field();
        let mut observer = trigger("observer", 13, 2, Vec::new());
        observer.events.push(EventDef {
            kind: 36,
            parameter: 3,
            ..EventDef::default()
        });
        world
            .attach_mission(mission(vec![
                trigger("set", 47, 1, vec![ActionDef::with_parameter(56, "3")]),
                trigger("set-again", 47, 2, vec![ActionDef::with_parameter(56, "3")]),
                observer,
            ]))
            .unwrap();
        for _ in 0..44 {
            world.tick();
        }
        assert_eq!(
            world.mission_state.as_ref().unwrap().triggers[2]
                .timer
                .start_tick,
            15
        );
        assert_eq!(fired(&world, "observer"), 0);
        world.tick();
        assert_eq!(fired(&world, "observer"), 1);
    }

    #[test]
    fn ra2_and_yr_local_variable_limits_are_distinct() {
        let mut data = mission(vec![trigger(
            "set",
            8,
            0,
            vec![ActionDef::with_parameter(56, "75")],
        )]);
        let mut ra2 = field();
        ra2.attach_mission(data.clone()).unwrap();
        ra2.tick();
        assert!(!ra2.mission_state.as_ref().unwrap().local(75));
        assert!(ra2
            .mission_state
            .as_ref()
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, DiagnosticKind::InvalidParameter { kind: 56, .. })));
        data.local_variable_limit = 100;
        let mut yr = field();
        yr.attach_mission(data).unwrap();
        yr.tick();
        assert!(yr.mission_state.as_ref().unwrap().local(75));
    }

    #[test]
    fn repeat_two_resets_the_shared_timer_and_once_destroys_the_whole_tag() {
        let mut world = field();
        let mut data = mission(vec![
            trigger("repeat", 13, 1, Vec::new()),
            trigger("once", 13, 1, Vec::new()),
        ]);
        data.tags[0].persistence = 2;
        world.attach_mission(data).unwrap();
        for _ in 0..46 {
            world.tick();
        }
        assert_eq!(fired(&world, "repeat"), 3);
        assert_eq!(fired(&world, "once"), 1);
        assert_eq!(
            world.mission_state.as_ref().unwrap().triggers[0]
                .timer
                .start_tick,
            45
        );
    }

    #[test]
    fn linked_triggers_share_the_tag_but_have_independent_timers() {
        let mut world = field();
        let mut data = mission(vec![
            trigger("first", 13, 1, Vec::new()),
            trigger("linked", 13, 2, Vec::new()),
        ]);
        data.triggers[0].linked = Some("linked".into());
        data.tags.truncate(1);
        data.tags[0].persistence = 2;
        world.attach_mission(data).unwrap();
        for _ in 0..30 {
            world.tick();
        }
        assert_eq!(fired(&world, "first"), 2);
        assert_eq!(fired(&world, "linked"), 1);
        assert_eq!(
            world.mission_state.as_ref().unwrap().tags[0].triggers.len(),
            2
        );
    }

    #[test]
    fn enable_updates_each_live_instance_of_a_trigger_type() {
        let mut world = field();
        let mut target = trigger("target", 13, 1, Vec::new());
        target.enabled = false;
        let mut data = mission(vec![
            target,
            trigger(
                "source",
                8,
                0,
                vec![ActionDef::with_parameter(53, "target")],
            ),
        ]);
        data.tags.push(TagDef {
            id: "second-target".into(),
            trigger: "target".into(),
            ..TagDef::default()
        });
        world.attach_mission(data).unwrap();
        for _ in 0..16 {
            world.tick();
        }
        assert_eq!(fired(&world, "target"), 2);
    }

    #[test]
    fn unsupported_active_paths_report_once_and_disabled_paths_stay_quiet() {
        let mut world = field();
        let mut disabled = trigger("disabled", 999, 0, Vec::new());
        disabled.enabled = false;
        let mut data = mission(vec![
            trigger("event", 999, 0, Vec::new()),
            trigger("action", 8, 0, vec![ActionDef::with_parameter(999, "0")]),
            disabled,
        ]);
        // Unknown object-only types have no periodic source on their own. A
        // known global event keeps this unsupported active path observable.
        data.triggers[0].events.push(EventDef { kind: 8, ..EventDef::default() });
        data.tags[0].persistence = 2;
        data.tags[1].persistence = 2;
        world.attach_mission(data).unwrap();
        for _ in 0..10 {
            world.tick();
        }
        let diagnostics = &world.mission_state.as_ref().unwrap().diagnostics;
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| matches!(d.kind, DiagnosticKind::UnsupportedEvent { .. }))
                .count(),
            1
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| matches!(d.kind, DiagnosticKind::UnsupportedAction { .. }))
                .count(),
            1
        );
        assert!(!diagnostics
            .iter()
            .any(|d| d.trigger_id.as_deref() == Some("disabled")));
        assert_eq!(fired(&world, "event"), 0);
    }

    #[test]
    fn a_mid_mission_snapshot_continues_identically_with_pending_outputs() {
        let mut data = mission(vec![trigger(
            "repeat",
            13,
            2,
            vec![
                ActionDef::with_parameter(56, "3"),
                ActionDef::with_parameter(19, "MessageText"),
            ],
        )]);
        data.tags[0].persistence = 2;
        let mut world = field();
        world.attach_mission(data.clone()).unwrap();
        for _ in 0..43 {
            world.tick();
        }
        let saved = world.snapshot();
        let mut restored = field();
        restored.attach_mission(data).unwrap();
        restored.restore(&saved).unwrap();
        assert_eq!(world.snapshot(), restored.snapshot());
        for _ in 0..150 {
            world.tick();
            restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
        }
        assert_eq!(
            world.take_mission_outputs(),
            restored.take_mission_outputs()
        );
    }

    #[test]
    fn a_save_requires_the_same_mission_rules_and_a_cycle_does_not_replace_the_world() {
        let data = mission(vec![trigger("first", 13, 1, Vec::new())]);
        let mut world = field();
        world.attach_mission(data.clone()).unwrap();
        let saved = world.snapshot();
        assert!(field().restore(&saved).is_err());
        let mut other = field();
        let mut changed = data.clone();
        changed.triggers[0].events[0].parameter = 2;
        other.attach_mission(changed).unwrap();
        assert!(other.restore(&saved).is_err());
        let before = world.snapshot();
        let mut cyclic = data;
        cyclic.triggers[0].linked = Some("first".into());
        assert!(world.attach_mission(cyclic).is_err());
        assert_eq!(before, world.snapshot());
    }

    #[test]
    fn win_resolves_country_index_rather_than_runtime_house_index() {
        let mut world = field();
        world.houses[1].country = 13;
        let mut data = mission(vec![trigger(
            "winner",
            8,
            0,
            vec![ActionDef::with_parameter(1, "13")],
        )]);
        data.country_names = (0..14).map(|index| format!("Country-{index}")).collect();
        data.country_names[13] = "Player".into();
        data.savour_delay_ticks = 0;
        world.end_checks = true;
        world.attach_mission(data).unwrap();
        assert!(!world.end_checks);
        world.tick();
        assert_eq!(
            world.mission_state.as_ref().unwrap().outcome,
            Some(MissionOutcome::Victory { house: 1 })
        );
        assert_eq!(world.over, Some(vec![1]));
    }

    #[test]
    fn win_lose_compare_current_player_country_even_for_unbound_indices_and_an_earlier_same_country_house() {
        for yuri in [false, true] { for action in [1, 2] {
            for parameter in [i32::MIN, -1, 0, 1, 13, 99, 8997, i32::MAX] {
                let mut world = field();
                world.houses[0].country = 13;
                world.houses[1].country = 13;
                let mut data = mission(vec![trigger("result", 8, 0, vec![ActionDef::with_parameter(action, parameter.to_string())])]);
                data.yuri = yuri;
                data.savour_delay_ticks = 0;
                data.country_names = (0..14).map(|index| format!("Country-{index}")).collect();
                data.country_names[13] = "Player".into();
                data.houses.insert(0, HouseBinding { name: "Earlier Same Country".into(), country: "Player".into(), house: 0 });
                world.attach_mission(data).unwrap();
                world.tick();
                let victory = (action == 1) == (parameter == 13);
                let expected = if victory { MissionOutcome::Victory { house: 1 } } else { MissionOutcome::Defeat { house: 1 } };
                let state = world.mission_state.as_ref().unwrap();
                assert_eq!(state.outcome, Some(expected), "editionYR={yuri} action={action} parameter={parameter}");
                assert!(state.diagnostics.iter().all(|diagnostic| !matches!(diagnostic.kind, DiagnosticKind::MissingCountryHouse { .. } | DiagnosticKind::NonPlayerOutcome { .. })));
                assert_eq!(world.over, Some(if victory { vec![1] } else { vec![] }));
                assert_eq!(world.take_mission_outputs().iter().filter(|output| matches!(output, MissionOutput::Outcome(outcome) if *outcome == expected)).count(), 1);
            }
        }}
    }

    #[test]
    fn outcome_requests_match_original_house_flag_and_timer_oracle() {
        let mut cases = 0;
        for row in include_str!("../tests/data/outcome_house_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let cells: Vec<_> = row.split('\t').collect();
            let integer = |at: usize| cells[at].parse::<i32>().unwrap();
            let flags = integer(1);
            let mut endgame = MissionEndgame {
                winner: flags & 2 != 0, loser: flags & 4 != 0,
                timer: OutcomeTimer { start_tick: 17, length_ticks: 99 },
                ..MissionEndgame::default()
            };
            endgame.request(integer(0) == 1, flags & 1 != 0, integer(3) as u64, savour_delay_ticks(cells[2].parse().unwrap()));
            let winner_or_loser = if integer(0) == 1 { endgame.winner } else { endgame.loser };
            assert_eq!([winner_or_loser as i32, (flags & 1) | ((endgame.winner as i32) << 8) | ((endgame.loser as i32) << 16), endgame.timer.start_tick, endgame.timer.length_ticks], [integer(4), integer(5), integer(6), integer(7)], "{row}");
            cases += 1;
        }
        assert_eq!(cases, 224);
        assert_eq!(savour_delay_ticks(1. / 900.), 0);
        assert_eq!(savour_delay_ticks(f64::NAN), i32::MIN);
        assert_eq!(savour_delay_ticks(f64::INFINITY), i32::MIN);
        assert_eq!(savour_delay_ticks(-f64::INFINITY), i32::MIN);
    }

    #[test]
    fn outcome_request_message_gates_match_original_house_source_labels() {
        let mut cases = 0;
        for yuri in [false, true] {
            for row in include_str!("../tests/data/outcome_messages_native.tsv").lines().filter(|row| !row.starts_with('#')) {
                let cells: Vec<_> = row.split('\t').collect();
                let integer = |at: usize| cells[at].parse::<i32>().unwrap();
                let flags = integer(1);
                let human = integer(3);
                let mut world = field();
                world.houses[1].human = human & 2 != 0;
                let mut data = mission(vec![trigger("request", 8, 0, vec![ActionDef::with_parameter(integer(0), "0")])]);
                data.yuri = yuri;
                data.human_player_houses = Some(if human & 1 != 0 { vec![1] } else { vec![] });
                world.attach_mission(data.clone()).unwrap();
                world.tick = integer(2) as u64;
                world.over = (flags & 1 != 0).then(Vec::new);
                let mut state = world.mission_state.take().unwrap();
                state.outputs.clear();
                state.endgame.winner = flags & 2 != 0;
                state.endgame.loser = flags & 4 != 0;
                world.mission_action(&data, &mut state, &data.triggers[0], 0, 0, &data.triggers[0].actions[0]);
                let expected = if cells[4] == "-" { vec![] } else { vec![
                    MissionOutput::Text { label: cells[4].into() },
                    MissionOutput::Speech { name: cells[5].into() },
                ] };
                assert_eq!(state.outputs, expected, "editionYR={yuri} row={row}");
                cases += 1;
            }
        }
        assert_eq!(cases, 128);
    }

    #[test]
    fn outcome_announces_before_delay_expiry_and_never_repeats_for_rejected_requests_or_final_outcome() {
        for victory in [false, true] {
            let mut world = field();
            let action = if victory { 1 } else { 2 };
            let mut data = mission(vec![trigger("request", 8, 0, vec![ActionDef::with_parameter(action, "0")])]);
            data.tags[0].persistence = 2;
            world.attach_mission(data).unwrap();
            world.tick();
            assert!(world.over.is_none());
            assert!(world.mission_state.as_ref().unwrap().outcome.is_none());
            let presentation: Vec<_> = world.take_mission_outputs().into_iter().filter(|output| matches!(output, MissionOutput::Text { .. } | MissionOutput::Speech { .. })).collect();
            assert_eq!(presentation, vec![
                MissionOutput::Text { label: if victory { "TXT_SCENARIO_WON" } else { "TXT_SCENARIO_LOST" }.into() },
                MissionOutput::Speech { name: if victory { "EVA_MissionAccomplished" } else { "EVA_MissionFailed" }.into() },
            ]);
            for _ in 1..90 { world.tick(); }
            assert!(world.over.is_none());
            assert_eq!(world.mission_state.as_ref().unwrap().endgame.timer.start_tick, 1);
            world.tick();
            assert!(world.over.is_some());
            assert!(world.take_mission_outputs().into_iter().all(|output| !matches!(output, MissionOutput::Text { .. } | MissionOutput::Speech { .. })));
        }
    }

    #[test]
    fn outcome_expiry_matches_original_house_ai_timer_and_control_gates() {
        let mut cases = 0;
        for row in include_str!("../tests/data/outcome_expiry_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut world = field();
            world.houses[1].human = value[1] & 2 != 0;
            let mut data = mission(Vec::new());
            data.human_player_houses = Some(if value[1] & 1 != 0 { vec![1] } else { vec![] });
            world.attach_mission(data).unwrap();
            world.tick = value[4] as u64;
            let endgame = &mut world.mission_state.as_mut().unwrap().endgame;
            endgame.winner = value[0] & 1 != 0;
            endgame.loser = value[0] & 2 != 0;
            endgame.timer = OutcomeTimer { start_tick: value[2], length_ticks: value[3] };
            endgame.allow_win_blocks = value[5];
            world.run_mission_endgame();
            let state = world.mission_state.as_ref().unwrap();
            let win = matches!(state.outcome, Some(MissionOutcome::Victory { .. }));
            let lose = matches!(state.outcome, Some(MissionOutcome::Defeat { .. }));
            assert_eq!([state.endgame.winner as i32, state.endgame.loser as i32, win as i32, lose as i32], value[6..10], "{row}");
            cases += 1;
        }
        assert_eq!(cases, 288);
    }

    #[test]
    fn outcome_wait_uses_wall_time_without_advancing_the_logic_frame_and_resumes_from_snapshot() {
        for victory in [true, false] {
            let mut data = mission(vec![trigger("result", 8, 0, vec![ActionDef::with_parameter(if victory { 1 } else { 2 }, "0")])]);
            data.savour_delay_ticks = 3;
            let mut world = field();
            world.attach_mission(data.clone()).unwrap();
            world.set_mission_presentation_managed(true);
            world.tick();
            assert!(world.over.is_none());
            assert!(!world.mission_presentation_waiting());
            assert!(!world.mission_input_locked());
            world.tick(); world.tick();
            assert!(world.over.is_none());
            world.tick();
            assert_eq!(world.tick, 4);
            assert!(world.mission_presentation_waiting());
            assert!(world.mission_input_locked());
            assert!(world.over.is_none());
            let frozen = world.snapshot();
            for _ in 0..300 { world.tick(); }
            assert_eq!(world.snapshot(), frozen);
            assert!(!world.advance_mission_presentation_time(119, true));
            assert_eq!(world.tick, 4);
            let saved = world.snapshot();
            let mut restored = field();
            restored.attach_mission(data).unwrap();
            restored.restore(&saved).unwrap();
            assert!(restored.mission_presentation_waiting());
            assert!(restored.mission_input_locked());
            assert!(!world.advance_mission_presentation_time(0, true));
            assert!(world.advance_mission_presentation_time(1, true));
            assert!(restored.advance_mission_presentation_time(1, true));
            assert_eq!(world.snapshot(), restored.snapshot());
            assert!(!world.mission_input_locked());
            assert!(!restored.mission_input_locked());
            assert_eq!(world.tick, 4);
            let expected = if victory { MissionOutcome::Victory { house: 1 } } else { MissionOutcome::Defeat { house: 1 } };
            assert_eq!(world.mission_state.as_ref().unwrap().outcome, Some(expected));
            assert_eq!(world.mission_state.as_ref().unwrap().endgame.winner, false);
            assert_eq!(world.mission_state.as_ref().unwrap().endgame.loser, !victory);
            assert_eq!(world.take_mission_outputs().iter().filter(|output| matches!(output, MissionOutput::Outcome(outcome) if *outcome == expected)).count(), 1);
            assert!(!world.advance_mission_presentation_time(120, false));
            world.tick();
            assert_eq!(world.tick, 4);
        }
    }

    #[test]
    fn outcome_audio_idle_finishes_immediately_and_default_headless_has_no_extra_wait() {
        for managed in [false, true] {
            let mut world = field();
            let mut data = mission(vec![trigger("win", 8, 0, vec![ActionDef::with_parameter(1, "0")])]);
            data.savour_delay_ticks = 90;
            world.attach_mission(data).unwrap();
            world.set_mission_presentation_managed(managed);
            for _ in 0..90 { world.tick(); }
            assert_eq!(world.tick, 90);
            assert!(world.over.is_none());
            world.tick();
            assert_eq!(world.tick, 91);
            if managed {
                assert!(world.over.is_none());
                assert!(world.advance_mission_presentation_time(0, false));
            }
            assert_eq!(world.over, Some(vec![1]));
            assert_eq!(world.mission_state.as_ref().unwrap().outcome, Some(MissionOutcome::Victory { house: 1 }));
        }
    }

    #[test]
    fn pending_outcome_timer_saves_while_units_keep_moving_and_freezes_only_at_expiry() {
        let setup = || {
            let mut world = field();
            let id = world.add_unit((4, 4), 64, crate::UnitStats { strength: 100, speed: 6, ..crate::UnitStats::default() }, 1);
            world.order(crate::Order::Move { unit: id, x: 12, y: 4 });
            let mut data = mission(vec![trigger("win", 8, 0, vec![ActionDef::with_parameter(1, "0")])]);
            data.savour_delay_ticks = 30;
            world.attach_mission(data).unwrap();
            world
        };
        let mut world = setup();
        let start = (world.units[0].x, world.units[0].y);
        for _ in 0..10 { world.tick(); }
        assert!(world.over.is_none());
        assert!(world.mission_state.as_ref().unwrap().endgame.winner);
        assert_ne!((world.units[0].x, world.units[0].y), start);
        let mut restored = setup();
        restored.restore(&world.snapshot()).unwrap();
        for _ in 0..30 {
            world.tick(); restored.tick();
            assert_eq!(world.state_hash(), restored.state_hash());
        }
        assert_eq!(world.tick, 31);
        assert_eq!(world.over, Some(vec![1]));
        let done = world.snapshot();
        for _ in 0..30 { world.tick(); }
        assert_eq!(world.snapshot(), done);
    }

    #[test]
    fn lose_overrides_pending_win_and_repeated_requests_do_not_restart_the_delay() {
        for actions in [vec![1, 1, 2, 2, 1], vec![2, 1, 2]] {
            let mut world = field();
            let mut data = mission(vec![trigger("result", 8, 0, actions.into_iter().map(|kind| ActionDef::with_parameter(kind, "0")).collect())]);
            data.tags[0].persistence = 2;
            data.savour_delay_ticks = 3;
            world.attach_mission(data).unwrap();
            world.tick();
            let initial = world.mission_state.as_ref().unwrap().endgame.clone();
            assert!(!initial.winner && initial.loser);
            assert_eq!(initial.timer, OutcomeTimer { start_tick: 1, length_ticks: 3 });
            world.tick();
            assert_eq!(world.mission_state.as_ref().unwrap().endgame, initial);
            world.tick(); world.tick();
            assert_eq!(world.over, Some(vec![]));
            assert_eq!(world.tick, 4);
        }
    }

    fn notification_original_runtime(index: u8) -> u8 { (index + 5) % 23 }

    fn notification_predicate_field(definition: TriggerDef, yuri: bool) -> (World, MissionData) {
        let mut world = field();
        world.houses = vec![House::default(); 23];
        world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
        let mut data = mission(vec![definition]);
        data.yuri = yuri;
        data.country_names = (0..23).map(|index| format!("Country{index}")).collect();
        data.houses = (0..23).map(|index| {
            let runtime = notification_original_runtime(index);
            let country = match index { 7 | 8 => 5, 22 => 13, _ => index };
            world.houses[runtime as usize].country = country;
            HouseBinding { name: format!("OriginalHouse{index}"), country: format!("Country{country}"), house: runtime }
        }).collect();
        data.cell_tags.push(((4, 4), format!("tag-{}", data.triggers[0].id)));
        (world, data)
    }

    #[test]
    fn event_six_matches_original_value_owner_source_and_repeat_inputs_without_remembering() {
        let mut cases = 0;
        for row in include_str!("../tests/data/event6_predicate_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut definition = trigger("six", 6, value[2], vec![]);
            definition.country = if value[5] == 0 { "Missing" } else { "Country5" }.into();
            let (mut world, mut data) = notification_predicate_field(definition, value[0] != 0);
            let mut object = |which: i32, cell| (which >= 0).then(|| crate::combat::Aim::Unit(world.add_unit(cell, 0,
                crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, notification_original_runtime([7, 8, 22][which as usize]))));
            let victim = object(value[3], (4, 4));
            let source = object(value[4], (6, 6));
            data.tags[0].persistence = if value[6] != 0 { 2 } else { 0 };
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            state.event_houses[0].1[0] = Some(1); // Preserve a nonnull existing result, as in the native oracle.
            let result = world.mission_conditions(&data, &mut state, 0, MissionEventContext { kind: value[1], object: victim, source, cell: None });
            assert_eq!(result, value[7] != 0, "{row}");
            assert_eq!(state.event_houses[0].1[0], Some(1), "Event6 must not write either owner's House: {row}");
            assert!(!state.triggers[0].occurred[0], "StateB=false must recheck6: {row}");
            cases += 1;
        }
        assert_eq!(cases, 3024);
    }

    #[test]
    fn event_forty_four_matches_original_source_array_index_with_permuted_runtime_houses() {
        let mut cases = 0;
        for row in include_str!("../tests/data/event44_predicate_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut definition = trigger("forty-four", 44, value[2], vec![]);
            definition.country = if value[4] == 0 { "Missing" } else { "Country5" }.into();
            let (mut world, mut data) = notification_predicate_field(definition, value[0] != 0);
            let source = (value[3] >= 0).then(|| {
                let owner = notification_original_runtime([7, 8, 22][value[3] as usize]);
                if value[3] == 1 { world.add_building(owner, 0, (6, 6)); crate::combat::Aim::Building(world.buildings.last().unwrap().id) }
                else { crate::combat::Aim::Unit(world.add_unit((6, 6), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, owner)) }
            });
            data.tags[0].persistence = if value[5] != 0 { 2 } else { 0 };
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            state.event_houses[0].1[0] = Some(1);
            let result = world.mission_conditions(&data, &mut state, 0, MissionEventContext { kind: value[1], object: None, source, cell: None });
            assert_eq!(result, value[6] != 0, "{row}");
            assert_eq!(state.event_houses[0].1[0], Some(1), "44 must not write its source/victim House: {row}");
            assert!(!state.triggers[0].occurred[0], "StateB=false cannot latch44: {row}");
            cases += 1;
        }
        assert_eq!(cases, 1344);
    }

    #[test]
    fn ordinary_damage_tail_rechecks_live_tag_and_life_against_original_mutation_oracle() {
        let mut cases = 0;
        for row in include_str!("../tests/data/ordinary_damage_tail_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<_> = row.split('\t').collect();
            let mut context = (value[2] == "1", (value[3] == "1").then_some(1u8), Vec::new());
            visit_ordinary_damage_notifications(&mut context, value[1].parse().unwrap(), value[4] == "1",
                |state| state.0, |state| state.1, |state, tag, kind| {
                    state.2.push((kind, tag));
                    if kind == 6 { match value[5] { "none" => {}, "clear_tag" => state.1 = None, "dead" => state.0 = false, "replace_tag" => state.1 = Some(2), _ => unreachable!() } }
                });
            let expected: Vec<(i32, u8)> = if value[6] == "-" { Vec::new() } else {
                value[6].split(',').map(|item| { let (kind, tag) = item.split_once('@').unwrap(); (kind.parse().unwrap(), tag.parse().unwrap()) }).collect()
            };
            assert_eq!(context.2, expected, "{row}");
            cases += 1;
        }
        assert_eq!(cases, 384);
    }

    #[test]
    fn successful_event_six_tag_detachment_matches_original_physical_reference_counts() {
        let mut cases = 0;
        for row in include_str!("../tests/data/event6_tag_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<u64> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut world = field();
            let mut data = mission(vec![trigger("six", 6, 999, vec![])]);
            data.yuri = true;
            data.tags[0].persistence = value[0] as u8;
            let mut owners = Vec::new();
            for n in 0..value[1] {
                let unit = world.add_unit((n as i32 + 3, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
                owners.push(crate::combat::Aim::Unit(unit));
                data.objects.push(ObjectBinding { section: "Units".into(), id: n.to_string(), tag: Some("tag-six".into()), object: crate::combat::Aim::Unit(unit), group: -1, initial_mission: "Guard".into() });
            }
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            world.raise_tag_mission_event(&data, &mut state, 0, MissionEventContext { kind: 6, object: Some(owners[0]), source: None, cell: None });
            assert_eq!(state.tag_refs(0) as u64, value[2], "{row}");
            let mask = owners.iter().enumerate().fold(0u64, |mask, (index, &owner)| mask | (u64::from(state.object_tag(owner).is_some()) << index));
            assert_eq!(mask, value[3], "{row}");
            assert_eq!(u64::from(state.triggers[0].destroyed), value[4], "{row}");
            assert_eq!(u64::from(state.triggers[0].occurred[0]), value[5], "{row}");
            assert_eq!(state.triggers[0].fire_count, value[6], "{row}");
            cases += 1;
        }
        assert_eq!(cases, 9);
    }

    fn ordinary_attack_field(yuri: bool, persistence: u8, refs: usize, include_six: bool) -> (World, crate::combat::Aim, crate::combat::Aim) {
        let mut world = field();
        world.houses.push(House { country: 5, ..House::default() });
        world.warheads.push(crate::combat::Warhead::default());
        let victim = crate::combat::Aim::Unit(world.add_unit((4, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1));
        let attacker = crate::combat::Aim::Unit(world.add_unit((8, 8), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 2));
        let mut definitions = vec![trigger("forty-four", 44, 0, vec![ActionDef::with_parameter(11, "forty-four")])];
        if include_six {
            let mut six = trigger("six", 6, 999, vec![ActionDef::with_parameter(11, "six")]);
            six.linked = Some("forty-four".into());
            definitions.insert(0, six);
        }
        let mut data = mission(definitions);
        data.yuri = yuri;
        data.tags.truncate(1);
        data.tags[0].persistence = persistence;
        data.houses.insert(0, HouseBinding { name: "Attacker".into(), country: "Country5".into(), house: 2 });
        for n in 0..refs {
            let object = if n == 0 { victim } else { crate::combat::Aim::Unit(world.add_unit((n as i32 + 10, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1)) };
            data.objects.push(ObjectBinding { section: "Units".into(), id: n.to_string(), tag: Some(data.tags[0].id.clone()), object, group: -1, initial_mission: "Guard".into() });
        }
        world.attach_mission(data).unwrap();
        (world, victim, attacker)
    }

    #[test]
    fn real_nonlethal_damage_orders_six_before_forty_four_and_honours_tag_detachment() {
        for yuri in [false, true] { for persistence in [0, 1, 2] { for refs in [1, 2, 3] { for include_six in [false, true] {
            let (mut world, victim, attacker) = ordinary_attack_field(yuri, persistence, refs, include_six);
            assert!(!world.receive_damage(victim, 10, 0, 0, Some(attacker)));
            let state = world.mission_state.as_ref().unwrap();
            let six = state.triggers.iter().find(|trigger| trigger.id == "six").map_or(0, |trigger| trigger.fire_count);
            let forty_four = fired(&world, "forty-four");
            let expected_fire = u64::from(persistence != 1 || refs == 1);
            assert_eq!(six, if include_six { expected_fire } else { 0 });
            assert_eq!(forty_four, if include_six && persistence != 2 { 0 } else { expected_fire }, "YR={yuri} p={persistence} refs={refs} six={include_six}");
            let labels: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
            let mut expected = Vec::new();
            if include_six && expected_fire != 0 { expected.push("six"); }
            if forty_four != 0 { expected.push("forty-four"); }
            assert_eq!(labels, expected);
        }}}}
    }

    #[test]
    fn zero_healing_null_source_and_lethal_damage_do_not_use_the_ordinary_six_forty_four_tail() {
        for damage in [-10, 0, 10, 1000] {
            let (mut world, victim, attacker) = ordinary_attack_field(true, 2, 1, true);
            let source = (damage != 10).then_some(attacker);
            let mut data = world.mission_data.take().unwrap();
            data.triggers.iter_mut().find(|trigger| trigger.id == "forty-four").unwrap().linked = Some("seven".into());
            let mut seven = trigger("seven", 7, 0, vec![]);
            seven.linked = Some("forty-eight".into());
            data.triggers.extend([seven, trigger("forty-eight", 48, 0, vec![])]);
            world.attach_mission(data).unwrap();
            assert_eq!(world.receive_damage(victim, damage, 0, 0, source), damage == 1000);
            assert_eq!((fired(&world, "six"), fired(&world, "forty-four")), (0, 0));
            assert_eq!((fired(&world, "seven"), fired(&world, "forty-eight")), if damage == 1000 { (2, 1) } else { (0, 0) }, "Persistent7 is remembered and fires again on the existing later48 source");
        }
    }

    #[test]
    fn forty_four_unbound_source_house_is_diagnosed_and_minus_one_is_not_a_wildcard() {
        for unbound in [false, true] {
            let (mut world, victim, attacker) = ordinary_attack_field(true, 2, 1, true);
            let mut data = world.mission_data.take().unwrap();
            if unbound { data.houses.remove(0); }
            else { data.triggers.iter_mut().find(|trigger| trigger.id == "forty-four").unwrap().events[0].parameter = -1; }
            world.attach_mission(data).unwrap();
            world.receive_damage(victim, 10, 0, 0, Some(attacker));
            assert_eq!((fired(&world, "six"), fired(&world, "forty-four")), (1, 0));
            assert_eq!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|item| matches!(item.kind, DiagnosticKind::UnsupportedEvent { kind: 44, .. })), unbound);
        }
    }

    #[test]
    fn ordinary_notifications_accept_registered_cargo_attackers_and_building_technos() {
        for building in [false, true] {
            let (mut world, victim, attacker) = ordinary_attack_field(true, 2, 1, false);
            let source = if building {
                world.building_types.push(crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() });
                world.add_building(2, 0, (10, 10));
                crate::combat::Aim::Building(world.buildings.last().unwrap().id)
            } else {
                let crate::combat::Aim::Unit(attacker) = attacker else { unreachable!() };
                let passenger = world.units.remove(world.units.iter().position(|unit| unit.id == attacker).unwrap());
                world.add_unit((10, 10), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 2);
                world.units.last_mut().unwrap().cargo.push(passenger);
                crate::combat::Aim::Unit(attacker)
            };
            world.receive_damage(victim, 10, 0, 0, Some(source));
            assert_eq!(fired(&world, "forty-four"), 1);
        }
    }

    #[test]
    fn ordinary_damage_notifications_and_live_tag_restore_without_a_new_snapshot_layout() {
        let (mut world, victim, attacker) = ordinary_attack_field(true, 2, 1, true);
        world.receive_damage(victim, 10, 0, 0, Some(attacker));
        let saved = world.snapshot();
        let (mut restored, _, _) = ordinary_attack_field(true, 2, 1, true);
        restored.restore(&saved).unwrap();
        for _ in 0..3 { world.receive_damage(victim, 10, 0, 0, Some(attacker)); restored.receive_damage(victim, 10, 0, 0, Some(attacker)); }
        assert_eq!(world.state_hash(), restored.state_hash());
        assert_eq!((fired(&world, "six"), fired(&world, "forty-four")), (4, 4));
    }

    #[test]
    fn mind_control_pre_capture_notification_stays_explicitly_unverified() {
        let (mut world, victim, attacker) = ordinary_attack_field(true, 2, 1, true);
        let warhead = crate::combat::Warhead { mind_control: true, ..crate::combat::Warhead::default() };
        assert!(world.special_hit(&warhead, Some(attacker), 2, victim, &crate::combat::Weapon { damage: 3, ..crate::combat::Weapon::default() }));
        assert_eq!((fired(&world, "six"), fired(&world, "forty-four")), (0, 0));
        assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|item| matches!(&item.kind, DiagnosticKind::ChangeHouseUnverified { feature, .. } if feature == "MindControlDamageNotification")));
    }

    #[test]
    fn building_count_predicates_match_original_house_counter_and_null_house_results() {
        let mut cases = 0;
        for row in include_str!("../tests/data/building_count_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut world = field();
            world.houses.push(House::default());
            world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }; 8];
            for kind in [0, 1, 7] { for n in 0..value[5] { world.add_building(1, kind, (kind as i32 + 1, n + 1)); }}
            for kind in [0, 1, 7] { for n in 0..2 { world.add_building(2, kind, (kind as i32 + 1, n + 4)); }}
            let mut data = mission(vec![trigger("owned", value[1], value[4], vec![])]);
            data.yuri = value[0] != 0;
            data.tags[0].persistence = if value[6] != 0 { 2 } else { 0 };
            let country = format!("CounterCountry{}", value[3]);
            data.triggers[0].country = country.clone();
            if value[3] != 0 {
                if let Some(binding) = data.houses.iter_mut().find(|binding| binding.house == value[3] as u8) { binding.country = country; }
                else { data.houses.push(HouseBinding { name: country.clone(), country, house: value[3] as u8 }); }
            }
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            // The direct native fixture's House input is reconstructed through
            // RegisterEvent's static Country lookup, never its YR output slot.
            state.triggers[0].house = Some(if value[3] == 1 { 2 } else { 1 });
            let result = world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: value[2], object: None, cell: None });
            assert_eq!(result, value[7] != 0, "{row}");
            assert!(!state.triggers[0].occurred[0], "Event32/57 must be rechecked rather than remembered: {row}");
            cases += 1;
        }
        assert_eq!(cases, 720);
    }

    #[test]
    fn building_count_register_country_and_runtime_house_outputs_match_original_complete_caller_chain() {
        let mut cases = 0;
        for row in include_str!("../tests/data/building_count_register_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut world = field();
            world.houses[1].country = 5;
            world.houses.push(House { country: if value[5] != 0 { 5 } else { 7 }, ..House::default() });
            world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
            for n in 0..value[7] { world.add_building(1, 0, (4, n + 1)); }
            for n in 0..2 { world.add_building(2, 0, (6, n + 1)); }
            let mut definition = trigger("register-owned", value[1], 0, vec![]);
            definition.country = format!("Country{}", value[3]);
            let mut data = mission(vec![definition]);
            data.yuri = value[0] != 0;
            data.houses = vec![
                HouseBinding { name: "First House".into(), country: "Country5".into(), house: 1 },
                HouseBinding { name: "Second House".into(), country: if value[5] != 0 { "Country5" } else { "Country7" }.into(), house: 2 },
            ];
            if value[4] != 0 { data.houses.reverse(); }
            data.tags[0].persistence = if value[8] != 0 { 2 } else { 0 };
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            let original_output = (value[6] != 0).then_some(value[6] as u8);
            state.triggers[0].house = original_output;
            let result = world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: value[2], object: None, cell: None });
            assert_eq!(result, value[9] != 0, "{row}");
            assert_eq!(state.triggers[0].house, original_output, "This event has no event.GetHouse output: {row}");
            assert!(!state.triggers[0].occurred[0], "Native StateA does not remember Event32/57: {row}");
            cases += 1;
        }
        assert_eq!(cases, 1152);
    }

    #[test]
    fn global_techno_predicates_match_original_case_sensitive_lookup_empty_array_and_signed_threshold() {
        let mut cases = 0;
        for row in include_str!("../tests/data/techno_count_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let cells: Vec<_> = row.split('\t').collect();
            let integer = |at: usize| cells[at].parse::<i32>().unwrap();
            let mut world = field();
            world.unit_types = vec![crate::UnitType::default(); 2];
            let objects: &[u16] = match cells[5] { "empty" => &[], "other" => &[1], "first" | "last" => &[2], "mixed" => &[1, 2, 2], _ => unreachable!() };
            for (i, kind) in objects.iter().copied().enumerate() {
                let id = world.add_unit((i as i32 + 1, 1), 0, crate::UnitStats { kind, strength: 100, ..crate::UnitStats::default() }, (i % 2) as u8);
                if cells[5] == "last" { world.units_mut_for_tests(id, -1); }
            }
            let mut definition = trigger("global", integer(0), integer(3), vec![]);
            definition.events[0].argument = cells[4].into();
            let mut data = mission(vec![definition]);
            data.yuri = true;
            data.techno_types = vec![TechnoTypeBinding { id: "type".into(), kind: MissionTechnoKind::Unit(0) }, TechnoTypeBinding { id: "TYPE".into(), kind: MissionTechnoKind::Unit(1) }];
            data.tags[0].persistence = if integer(6) != 0 { 2 } else { 0 };
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            state.triggers[0].house = (integer(2) != 0).then_some(1);
            let result = world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: integer(1), object: None, cell: None });
            assert_eq!(result, integer(7) != 0, "{row}");
            assert!(!state.triggers[0].occurred[0]);
            cases += 1;
        }
        assert_eq!(cases, 1200);
    }

    #[test]
    fn building_exists_uses_static_country_preserves_registered_dead_objects_and_obeys_edition_dontscore() {
        for yuri in [false, true] {
            let mut world = field();
            world.houses.push(House::default());
            world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
            world.add_building(2, 0, (4, 4));
            world.buildings[0].health = 0;
            let mut data = mission(vec![trigger("owned", 32, 0, vec![])]);
            data.yuri = yuri;
            data.triggers[0].country = "Player".into();
            data.houses.push(HouseBinding { name: "Enemy House".into(), country: "Enemy".into(), house: 2 });
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            let context = MissionEventContext { source: None, kind: 13, object: None, cell: None };
            state.triggers[0].house = Some(2);
            assert!(!world.mission_conditions(&data, &mut state, 0, context));
            data.triggers[0].country = "Enemy".into();
            state.triggers[0].house = Some(1);
            assert!(world.mission_conditions(&data, &mut state, 0, context), "Health0 still registered; YR={yuri}");
            data.building_dont_score = vec![0];
            assert_eq!(world.mission_conditions(&data, &mut state, 0, context), !yuri);
            world.buildings.clear();
            assert!(!world.mission_conditions(&data, &mut state, 0, context));
        }
    }

    #[test]
    fn techno_counts_include_cargo_and_current_factory_objects_while_owned_buildings_exclude_factory_limbo() {
        let mut world = field();
        world.unit_types = vec![crate::UnitType::default(); 2];
        world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
        world.add_unit((4, 4), 0, crate::UnitStats { kind: 1, strength: 100, ..crate::UnitStats::default() }, 1);
        let passenger = world.units.pop().unwrap();
        let carrier = world.add_unit((5, 5), 0, crate::UnitStats { kind: 2, strength: 100, ..crate::UnitStats::default() }, 1);
        world.units.iter_mut().find(|unit| unit.id == carrier).unwrap().cargo.push(passenger);
        assert_eq!(world.registered_techno_count(MissionTechnoKind::Unit(0)), (1, 2));
        let product = crate::Production { building: 0, stage: 0, rate: 0, next: 0, balance: 100, suspended: true, stalled: false };
        world.houses[1].production = Some(product);
        world.houses[1].vehicles = Some(product);
        world.houses[1].vehicle_queue = vec![0, 0, 0];
        assert_eq!(world.factory_techno_count(MissionTechnoKind::Unit(0)), (1, 2));
        assert_eq!(world.factory_techno_count(MissionTechnoKind::Building(0)), (1, 2));
        assert!(world.buildings.is_empty(), "Current Factory objects have not entered OwnedBuildingTypes");
        world.houses[1].vehicles.as_mut().unwrap().stage = crate::PRODUCTION_STAGES;
        assert_eq!(world.factory_techno_count(MissionTechnoKind::Unit(0)), (1, 2), "Ready and held current products still exist, queues do not");
        world.houses[1].vehicles = None;
        assert_eq!(world.factory_techno_count(MissionTechnoKind::Unit(0)), (0, 1));
    }

    #[test]
    fn building_count_success_changes_shared_repeat_for_later_destruction_but_missing_country_does_not() {
        for kind in [32, 57] { for house in [None, Some(1)] {
            let mut world = field();
            world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
            if kind == 32 { world.add_building(1, 0, (4, 4)); }
            let mut definition = trigger("and", kind, 0, vec![]);
            definition.events.push(EventDef { kind: 48, ..EventDef::default() });
            definition.country = if house.is_some() { "Player" } else { "Missing" }.into();
            let data = mission(vec![definition]);
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            state.triggers[0].house = if house.is_some() { None } else { Some(1) };
            assert!(world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: 48, object: None, cell: None }));
            assert!(!state.triggers[0].occurred[0]);
            assert_eq!(state.triggers[0].occurred[1], house.is_some());
        }}
    }

    #[test]
    fn unsafe_building_indices_and_ambiguous_registry_ids_remain_diagnostic_and_unsatisfied() {
        let mut world = field();
        world.unit_types = vec![crate::UnitType::default(); 2];
        let mut negative = trigger("negative", 57, -1, vec![]);
        negative.country = "Player".into();
        negative.events[0].argument.clear();
        let mut ambiguous = trigger("ambiguous", 61, 0, vec![]);
        ambiguous.events[0].argument = "TYPE".into();
        let mut data = mission(vec![negative, ambiguous]);
        data.yuri = true;
        data.techno_types = vec![TechnoTypeBinding { id: "TYPE".into(), kind: MissionTechnoKind::Unit(0) }, TechnoTypeBinding { id: "TYPE".into(), kind: MissionTechnoKind::Unit(1) }];
        world.attach_mission(data.clone()).unwrap();
        let mut state = world.mission_state.take().unwrap();
        let context = MissionEventContext { source: None, kind: 13, object: None, cell: None };
        state.triggers[0].house = Some(1);
        for index in 0..2 { assert!(!world.mission_conditions(&data, &mut state, index, context)); }
        assert!(state.diagnostics.iter().any(|item| matches!(item.kind, DiagnosticKind::InvalidParameter { kind: 57, .. })));
        assert!(state.diagnostics.iter().any(|item| matches!(item.kind, DiagnosticKind::UnsupportedEvent { kind: 61, .. })));
    }

    #[test]
    fn house_source_live_reverse_indices_match_original_cpu_with_callback_removals() {
        let mut cases = 0;
        for row in include_str!("../tests/data/house_tag_source_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let cells: Vec<_> = row.split('\t').collect();
            let frame: u64 = cells[1].parse().unwrap();
            let count: usize = cells[2].parse().unwrap();
            let mut model = ((1..=count as i32).collect::<Vec<_>>(), Vec::new(), false);
            let complete = visit_house_tags_live(frame, &mut model, count, |state, index| state.0.get(index).copied(), |state, tag| {
                state.1.push(tag);
                if !state.2 && cells[3] != "none" {
                    let index = if cells[3] == "lowest" { 0 } else { state.0.iter().position(|value| *value == tag).unwrap() };
                    state.0.remove(index);
                    state.2 = true;
                }
            });
            assert!(complete, "{row}");
            let expected: Vec<i32> = if cells[4] == "-" { vec![] } else { cells[4].split(',').map(|value| value.parse().unwrap()).collect() };
            assert_eq!(model.1, expected, "{row}");
            cases += 1;
        }
        assert_eq!(cases, 72);
    }

    fn counted_house_source_world(persistence: u8) -> World {
        let mut world = field();
        world.houses.push(House { defeated: true, ..House::default() });
        world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
        world.add_building(1, 0, (4, 4));
        world.add_building(2, 0, (6, 6));
        let mut definitions = Vec::new();
        for (id, country) in [("alpha", "Player"), ("beta", "Player"), ("gamma", "Enemy"), ("linked", "Enemy")] {
            let mut definition = trigger(id, 32, 0, vec![ActionDef::with_parameter(11, id)]);
            definition.country = country.into();
            definitions.push(definition);
        }
        definitions[0].linked = Some("linked".into());
        let mut data = mission(definitions);
        data.tags.pop(); // A linked Trigger does not have an independent TagType.
        for tag in &mut data.tags { tag.persistence = persistence; }
        data.houses.insert(0, HouseBinding { name: "Enemy House".into(), country: "Enemy".into(), house: 2 });
        data.passive_houses = vec![2];
        world.attach_mission(data).unwrap();
        world
    }

    #[test]
    fn house_sources_poll_eighth_frames_in_original_house_order_and_reverse_map_tag_order_and_restore() {
        for persistence in [0, 2] {
            let mut world = counted_house_source_world(persistence);
            for _ in 0..7 { world.tick(); }
            assert!(world.take_mission_outputs().iter().all(|output| !matches!(output, MissionOutput::Text { .. })));
            world.tick();
            let labels: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
            assert_eq!(labels, ["gamma", "beta", "linked", "alpha"], "Human Player and defeated passive Enemy both receive native House source; linked Trigger keeps own House, registers only with FirstTrigger's House");
            let saved = world.snapshot();
            let mut restored = counted_house_source_world(persistence);
            restored.restore(&saved).unwrap();
            for _ in 0..16 {
                world.tick(); restored.tick();
                assert_eq!(world.state_hash(), restored.state_hash());
            }
            let count = world.take_mission_outputs().iter().filter(|output| matches!(output, MissionOutput::Text { .. })).count();
            assert_eq!(count, if persistence == 0 { 0 } else { 8 });
        }
    }

    #[test]
    fn house_source_registration_keeps_first_trigger_static_country_when_runtime_house_changes() {
        let mut world = counted_house_source_world(2);
        world.tick = 16;
        let state = world.mission_state.as_mut().unwrap();
        state.triggers.iter_mut().find(|trigger| trigger.id == "alpha").unwrap().house = Some(2);
        world.take_mission_outputs();
        world.run_house_mission_tags(2);
        let labels: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
        assert_eq!(labels, ["gamma"], "Runtime House does not move alpha into another House's RelatedTags");
        world.run_house_mission_tags(1);
        let labels: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
        assert_eq!(labels, ["beta", "linked", "alpha"]);
    }

    #[test]
    fn building_count_source_uses_static_country_with_fresh_null_runtime_house_and_after_restore() {
        for yuri in [false, true] {
            let make = || {
                let mut world = field();
                world.houses.push(House { country: 2, ..House::default() });
                world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
                world.add_building(2, 0, (6, 6));
                let mut present = trigger("present", 32, 0, vec![ActionDef::with_parameter(11, "present")]);
                let mut absent = trigger("absent", 57, 0, vec![ActionDef::with_parameter(11, "absent")]);
                present.country = "Player".into();
                absent.country = "Player".into();
                let mut data = mission(vec![present, absent]);
                data.yuri = yuri;
                data.houses.push(HouseBinding { name: "Enemy House".into(), country: "Enemy".into(), house: 2 });
                for tag in &mut data.tags { tag.persistence = 2; }
                world.attach_mission(data).unwrap();
                world
            };
            let mut world = make();
            assert!(world.mission_state.as_ref().unwrap().triggers.iter().all(|trigger| trigger.house.is_none()));
            for _ in 0..8 { world.tick(); }
            assert_eq!((fired(&world, "present"), fired(&world, "absent")), (0, 1), "A fresh null runtime output must not make both predicates true; YR={yuri}");
            let saved = world.snapshot();
            let mut restored = make();
            restored.restore(&saved).unwrap();
            for current in [&mut world, &mut restored] { current.add_building(1, 0, (4, 4)); }
            for _ in 0..8 { world.tick(); restored.tick(); }
            assert_eq!(world.state_hash(), restored.state_hash());
            assert_eq!((fired(&world, "present"), fired(&world, "absent")), (1, 1));
        }
    }

    #[test]
    fn entered_by_house_output_does_not_replace_static_country_for_a_later_building_event() {
        let mut world = field();
        world.houses[1].country = 1;
        world.houses.push(House { country: 2, ..House::default() });
        world.building_types = vec![crate::BuildingType { size: (1, 1), strength: 100, ..crate::BuildingType::default() }];
        world.add_building(2, 0, (6, 6));
        let unit = world.add_unit((4, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 2);
        let mut definition = trigger("entered-and-owned", 1, -1, vec![]);
        definition.country = "Player".into();
        definition.events.push(EventDef { kind: 32, parameter: 0, ..EventDef::default() });
        let mut data = mission(vec![definition]);
        data.yuri = true;
        data.cell_tags.push(((4, 4), "tag-entered-and-owned".into()));
        data.houses.push(HouseBinding { name: "Enemy House".into(), country: "Enemy".into(), house: 2 });
        world.attach_mission(data.clone()).unwrap();
        let mut state = world.mission_state.take().unwrap();
        let context = MissionEventContext { source: None, kind: 1, object: Some(crate::combat::Aim::Unit(unit)), cell: Some((4, 4)) };
        assert!(!world.mission_conditions(&data, &mut state, 0, context));
        assert_eq!(state.triggers[0].house, Some(2), "EnteredBy still writes its separate runtime output");
        world.add_building(1, 0, (3, 3));
        assert!(world.mission_conditions(&data, &mut state, 0, context));
    }

    #[test]
    fn country_lookup_uses_original_binding_order_instead_of_lowest_runtime_house() {
        let mut world = field();
        world.houses[1].country = 5;
        world.houses.push(House { country: 5, ..House::default() });
        let mut data = mission(vec![trigger("entered", 1, 5, vec![])]);
        data.cell_tags.push(((4, 4), "tag-entered".into()));
        data.houses.insert(0, HouseBinding { name: "First Original House".into(), country: "Player".into(), house: 2 });
        assert_eq!(data.country_house("Player"), Some(2));
        let unit = world.add_unit((4, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 2);
        world.attach_mission(data.clone()).unwrap();
        let mut state = world.mission_state.take().unwrap();
        assert!(world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: 1, object: Some(crate::combat::Aim::Unit(unit)), cell: None }));
    }

    #[test]
    fn power_full_raw_counter_predicate_matches_original_including_signed_boundaries() {
        let mut cases = 0;
        for row in include_str!("../tests/data/power_full_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            assert_eq!(native_power_full(value[0], value[1]), value[2] != 0, "{row}");
            cases += 1;
        }
        assert_eq!(cases, 11);
    }

    #[test]
    fn power_full_selects_first_country_house_independently_of_raised_kind_or_runtime_house() {
        let mut cases = 0;
        for row in include_str!("../tests/data/power_full_predicate_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut world = field();
            world.houses[0].country = 5;
            world.houses[1].country = 5;
            world.houses.push(House { country: 13, ..House::default() });
            world.building_types = [value[4], -value[5], value[5], -value[4]].into_iter().map(|power| crate::BuildingType { power, strength: 1, size: (1, 1), ..crate::BuildingType::default() }).collect();
            for house in 0..3 {
                let offset = if house == 1 { 2 } else { 0 };
                world.add_building(house, offset, (house as i32 + 2, 2));
                world.add_building(house, offset + 1, (house as i32 + 2, 4));
            }
            let mut data = mission(vec![trigger("power", 58, value[1], vec![])]);
            data.yuri = true;
            data.country_names = (0..14).map(|index| format!("Country{index}")).collect();
            data.houses = vec![HouseBinding { name: "First".into(), country: "Country5".into(), house: 0 }, HouseBinding { name: "Duplicate".into(), country: "Country5".into(), house: 1 }, HouseBinding { name: "Other".into(), country: "Country13".into(), house: 2 }];
            data.tags[0].persistence = if value[3] != 0 { 2 } else { 0 };
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            state.triggers[0].house = (value[2] != 0).then_some(1);
            let event_houses = state.event_houses.clone();
            let result = world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: value[0], object: None, cell: None });
            assert_eq!(result, value[6] != 0, "{row}");
            assert_eq!(state.event_houses, event_houses, "Event58 must not write its shared Event.House");
            assert!(!state.triggers[0].occurred[0], "Power condition stays live");
            cases += 1;
        }
        assert_eq!(cases, 960);
    }

    #[test]
    fn power_full_runs_on_the_house_source_and_unbound_country_is_native_success() {
        let mut world = field();
        world.houses[1].country = 1;
        world.houses.push(House { country: 2, ..House::default() });
        world.building_types = vec![crate::BuildingType { power: -50, strength: 100, size: (1, 1), ..crate::BuildingType::default() }, crate::BuildingType { power: 100, strength: 100, size: (1, 1), ..crate::BuildingType::default() }];
        world.add_building(1, 1, (2, 2));
        world.add_building(2, 0, (4, 4));
        let mut data = mission(vec![trigger("enemy-full", 58, 2, vec![ActionDef::with_parameter(11, "enemy-full")]), trigger("unbound", 58, 999, vec![ActionDef::with_parameter(11, "unbound")])]);
        data.yuri = true;
        data.country_names.push("Enemy".into());
        data.houses.push(HouseBinding { name: "Enemy House".into(), country: "Enemy".into(), house: 2 });
        for definition in &mut data.triggers { definition.country = "Player".into(); }
        world.attach_mission(data).unwrap();
        for _ in 0..8 { world.tick(); }
        assert_eq!(fired(&world, "enemy-full"), 0, "The triggering player's full power must not substitute for parameter Enemy");
        assert_eq!(fired(&world, "unbound"), 1);
        world.add_building(2, 1, (6, 6));
        for _ in 0..7 { world.tick(); }
        assert_eq!(fired(&world, "enemy-full"), 0);
        world.tick();
        assert_eq!(fired(&world, "enemy-full"), 1);
    }

    #[test]
    fn power_full_does_not_change_shared_repeat_and_ra2_does_not_inherit_the_yr_event() {
        let mut world = field();
        let mut definition = trigger("power", 58, 999, vec![]);
        definition.events.push(EventDef { kind: 48, ..EventDef::default() });
        let mut data = mission(vec![definition]);
        data.yuri = true;
        world.attach_mission(data.clone()).unwrap();
        let mut state = world.mission_state.take().unwrap();
        assert!(world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: 48, object: None, cell: None }));
        assert!(!state.triggers[0].occurred[1], "PowerFull success leaves the incoming repeat=false unchanged");
        data.yuri = false;
        assert!(!world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: 48, object: None, cell: None }));
        assert!(state.diagnostics.iter().any(|item| matches!(item.kind, DiagnosticKind::UnsupportedEvent { kind: 58, .. })));
    }

    #[test]
    fn no_mission_changes_neither_random_stream_nor_the_next_tick() {
        let mut world = field();
        let before = world.snapshot();
        world.run_mission();
        assert_eq!(world.snapshot(), before);
        let mut other = field();
        for _ in 0..100 {
            world.tick();
            other.tick();
        }
        assert_eq!(world.snapshot(), other.snapshot());
    }

    #[test]
    fn scenario_initialization_scales_the_actual_strength_and_preserves_rank() {
        let mut world = field();
        let id = world.add_unit(
            (2, 2),
            0,
            crate::UnitStats {
                strength: 240,
                trainable: true,
                ..crate::UnitStats::default()
            },
            1,
        );
        assert!(world.initialize_scenario_unit(id, 128, 1.5));
        let unit = world.unit(id).unwrap();
        assert_eq!(unit.health, 120);
        assert_eq!(unit.veterancy, 1.5);
        assert_eq!(unit.rank(), 1);
        assert!(!world.initialize_scenario_unit(id, 128, f64::NAN));
        assert_eq!(world.unit(id).unwrap().veterancy, 1.5);
    }

    #[test]
    fn original_action_waypoints_keep_decimal_and_letter_arguments_distinct() {
        assert_eq!(decode_waypoint("A"), Some(0));
        assert_eq!(decode_waypoint("Z"), Some(25));
        assert_eq!(decode_waypoint("AA"), Some(26));
        assert_eq!(decode_waypoint("GM"), Some(194));
        assert_eq!(decode_waypoint("-1"), None);
        let mut world = field();
        world.map.shroud_on = true;
        let mut camera = ActionDef::with_parameter(48, "3");
        camera.parameters[6] = "B".into();
        let mut data = mission(vec![trigger(
            "startup",
            8,
            0,
            vec![ActionDef::with_parameter(17, "1"), camera],
        )]);
        data.waypoints = vec![(1, (8, 8))];
        world.attach_mission(data).unwrap();
        world.tick();
        assert!(world.map.revealed_to(1, 8, 8));
        assert!(world
            .take_mission_outputs()
            .contains(&MissionOutput::Camera {
                speed: 3,
                waypoint: 1,
                cell: (8, 8)
            }));
    }

    #[test]
    fn campaign_house_control_uses_human_or_player_control_without_changing_other_sessions() {
        let mut world = field();
        world.houses.push(House { human: true, ..House::default() });
        assert!(world.house_controlled_by(1, 1));
        assert!(!world.house_controlled_by(2, 1));
        assert!(world.house_controlled_by(0, 0), "other sessions compare the current House even if its Human bit is false");
        world.attach_mission(mission(Vec::new())).unwrap();
        assert!(world.house_controlled_by(1, 1));
        assert!(world.house_controlled_by(2, 1));
        assert!(!world.house_controlled_by(0, 1));
        assert!(!world.house_controlled_by(9, 1));
    }

    #[test]
    fn input_lock_preserves_script_orders_ticks_and_snapshot_resume_until_unlock() {
        fn setup() -> (World, u32) {
            let mut world = field();
            let unit = world.add_unit((3, 4), 0, crate::UnitStats { strength: 100, speed: 6, ..crate::UnitStats::default() }, 1);
            world.attach_mission(mission(vec![
                trigger("lock", 8, 0, vec![ActionDef::with_parameter(46, "0")]),
                trigger("unlock", 13, 1, vec![ActionDef::with_parameter(47, "0")]),
            ])).unwrap();
            (world, unit)
        }
        let (mut world, unit) = setup();
        assert!(!world.mission_input_locked());
        world.tick();
        assert!(world.mission_input_locked());
        world.order(crate::Order::Move { unit, x: 10, y: 4 });
        for _ in 0..6 { world.tick(); }
        assert!(world.unit(unit).unwrap().is_moving(), "input lock must not freeze scripted orders");
        let saved = world.snapshot();
        let (mut restored, _) = setup();
        restored.restore(&saved).unwrap();
        assert!(restored.mission_input_locked());
        for _ in 0..25 {
            world.tick();
            restored.tick();
            assert_eq!(world.state_hashes(), restored.state_hashes());
            assert_eq!(world.mission_input_locked(), world.current_tick() < 15);
        }
        assert_eq!((fired(&world, "lock"), fired(&world, "unlock")), (1, 1));
    }

    fn move_until_still(world: &mut World, unit: u32, cell: (i32, i32)) {
        world.order(crate::Order::Move { unit, x: cell.0, y: cell.1 });
        for _ in 0..500 {
            world.tick();
            if world.unit(unit).is_some_and(|unit| unit.cell() == cell && !unit.is_moving()) { return; }
        }
        panic!("unit {unit} did not reach {cell:?}: {:?}", world.unit(unit));
    }

    #[test]
    fn entered_by_uses_first_country_house_and_fires_only_on_completed_reentries() {
        for infantry in [false, true] {
            for yuri in [false, true] {
                let mut world = field();
                world.houses[1].country = 13;
                world.houses.push(House { country: 13, human: true, ..House::default() });
                let stats = crate::UnitStats { infantry, strength: 100, speed: 6, ..crate::UnitStats::default() };
                let player = if infantry { world.add_infantry((4, 6), 2, 64, stats, 1).unwrap() } else { world.add_unit((4, 6), 64, stats, 1) };
                let duplicate_country = if infantry { world.add_infantry((6, 8), 2, 0, stats, 2).unwrap() } else { world.add_unit((6, 8), 0, stats, 2) };
                let mut data = mission(vec![trigger("entered", 1, 13, vec![ActionDef::with_parameter(11, "reached")])]);
                data.yuri = yuri;
                data.houses.push(HouseBinding { name: "Second Player".into(), country: "Player".into(), house: 2 });
                data.tags[0].persistence = 2;
                data.cell_tags = vec![((6, 6), "tag-entered".into())];
                world.attach_mission(data).unwrap();
                move_until_still(&mut world, duplicate_country, (6, 6));
                assert_eq!(fired(&world, "entered"), 0, "second House of same Country is not FindByCountryIndex's result");
                move_until_still(&mut world, duplicate_country, (10, 8));
                world.order(crate::Order::Move { unit: player, x: 6, y: 6 });
                let mut crossed_before_completion = false;
                for _ in 0..500 {
                    world.tick();
                    let unit = world.unit(player).unwrap();
                    let completed = if infantry { unit.head_to.is_none() } else { unit.drive.is_none() };
                    if unit.cell() == (6, 6) && !completed {
                        crossed_before_completion = true;
                        assert_eq!(fired(&world, "entered"), 0, "cell boundary is not PCP.End");
                    }
                    if fired(&world, "entered") == 1 { break; }
                }
                assert!(crossed_before_completion);
                assert_eq!(fired(&world, "entered"), 1);
                for _ in 0..20 { world.tick(); }
                assert_eq!(fired(&world, "entered"), 1, "standing still is not a source");
                move_until_still(&mut world, player, (4, 6));
                move_until_still(&mut world, player, (6, 6));
                assert_eq!(fired(&world, "entered"), 2);
                let state = world.mission_state.as_ref().unwrap();
                assert!(!state.triggers[0].occurred[0], "EnteredBy has StateA but not StateB and must not latch");
                assert_eq!(state.event_houses[0].1[0], yuri.then_some(1));
                assert_eq!(state.triggers[0].house, yuri.then_some(1));
            }
        }
    }

    #[test]
    fn entered_overflown_predicate_matches_original_country_owner_and_repeat_matrix() {
        let mut cases = 0;
        for row in include_str!("../tests/data/entered_overflown_predicate_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut world = field();
            world.houses[0].country = 5;
            world.houses[1].country = 5;
            world.houses.push(House { country: 13, ..House::default() });
            let object = (value[2] >= 0).then(|| world.add_unit((4, 4), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, value[2] as u8));
            let mut definition = trigger("overflown", 59, value[1], vec![]);
            definition.country = if value[3] == 0 { "Missing" } else { "Country5" }.into();
            let mut data = mission(vec![definition]);
            data.yuri = true;
            data.country_names = (0..14).map(|index| format!("Country{index}")).collect();
            data.houses = vec![
                HouseBinding { name: "First".into(), country: "Country5".into(), house: 0 },
                HouseBinding { name: "Duplicate".into(), country: "Country5".into(), house: 1 },
                HouseBinding { name: "Other".into(), country: "Country13".into(), house: 2 },
            ];
            data.tags[0].persistence = if value[4] != 0 { 2 } else { 0 };
            data.cell_tags.push(((4, 4), "tag-overflown".into()));
            world.attach_mission(data.clone()).unwrap();
            let mut state = world.mission_state.take().unwrap();
            let result = world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: value[0], object: object.map(crate::combat::Aim::Unit), cell: None });
            assert_eq!(result, value[5] != 0, "{row}");
            let expected_house = (value[7] >= 0).then_some(value[7] as u8);
            assert_eq!(state.event_houses[0].1[0], expected_house, "{row}");
            assert_eq!(state.triggers[0].house, expected_house, "{row}");
            assert!(!state.triggers[0].occurred[0], "StateA=true / StateB=false cannot latch59: {row}");
            // Event48's latch observes the same writable repeat value after59.
            data.triggers[0].events.push(EventDef { kind: 48, ..EventDef::default() });
            state.triggers[0].occurred.push(false);
            state.event_houses[0].1.push(None);
            world.mission_conditions(&data, &mut state, 0, MissionEventContext { source: None, kind: 48, object: None, cell: None });
            assert_eq!(state.triggers[0].occurred[1], value[4] != 0, "A rejected59 leaves input repeat unchanged: {row}");
            cases += 1;
        }
        assert_eq!(cases, 768);
    }

    fn paired_cell_events(yuri: bool, persistence: u8) -> (World, u32) {
        let mut world = field();
        let unit = world.add_unit((3, 6), 64, crate::UnitStats { strength: 100, speed: 6, ..crate::UnitStats::default() }, 1);
        let mut first = trigger("entered", 1, -1, vec![ActionDef::with_parameter(11, "one")]);
        first.linked = Some("overflown".into());
        let second = trigger("overflown", 59, -1, vec![ActionDef::with_parameter(11, "fifty-nine")]);
        let mut data = mission(vec![first, second]);
        data.yuri = yuri;
        data.tags.pop();
        data.tags[0].persistence = persistence;
        data.cell_tags.push(((5, 6), "tag-entered".into()));
        world.attach_mission(data).unwrap();
        (world, unit)
    }

    #[test]
    fn ground_completion_raises_one_then_yr_fifty_nine_once_and_standing_is_not_a_source() {
        for yuri in [false, true] {
            let (mut world, unit) = paired_cell_events(yuri, 2);
            move_until_still(&mut world, unit, (5, 6));
            assert_eq!((fired(&world, "entered"), fired(&world, "overflown")), (1, u64::from(yuri)));
            let labels: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
            assert_eq!(labels, if yuri { vec!["one", "fifty-nine"] } else { vec!["one"] });
            for _ in 0..32 { world.tick(); }
            assert_eq!((fired(&world, "entered"), fired(&world, "overflown")), (1, u64::from(yuri)));
            if !yuri { assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|item| matches!(item.kind, DiagnosticKind::UnsupportedEvent { kind: 59, .. }))); }
        }
    }

    #[test]
    fn cached_ground_tag_keeps_fifty_nine_after_one_clears_cell_and_queues_deletion() {
        let (mut world, unit) = paired_cell_events(true, 0);
        move_until_still(&mut world, unit, (5, 6));
        assert_eq!((fired(&world, "entered"), fired(&world, "overflown")), (1, 1));
        let state = world.mission_state.as_ref().unwrap();
        assert!(state.tags[0].cells.is_empty());
        assert!(state.tags[0].destroyed);
        assert!(state.diagnostics.iter().any(|item| matches!(&item.kind, DiagnosticKind::CellEntryUnverified { feature } if feature == "CachedTagDeletionQueue")));
        let labels: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
        assert_eq!(labels, ["one", "fifty-nine"]);
    }

    #[test]
    fn live_tag_lookup_rejects_pending_but_cached_pointer_uses_native_destroyed_guard() {
        let (mut world, unit) = paired_cell_events(true, 0);
        let data = world.mission_data.take().unwrap();
        let mut state = world.mission_state.take().unwrap();
        let context = |kind| MissionEventContext { source: None, kind, object: Some(crate::combat::Aim::Unit(unit)), cell: Some((5, 6)) };
        world.raise_tag_mission_event(&data, &mut state, 0, context(1));
        assert!(state.tags[0].pending_delete && !state.tags[0].destroyed);
        assert_eq!(state.cell_tag((5, 6)), None);
        world.raise_tag_mission_event(&data, &mut state, 0, context(59));
        assert_eq!(state.triggers.iter().find(|trigger| trigger.id == "overflown").unwrap().fire_count, 0);
        world.raise_cached_tag_mission_event(&data, &mut state, 0, context(59));
        assert_eq!(state.triggers.iter().find(|trigger| trigger.id == "overflown").unwrap().fire_count, 1);
        state.destroy_tag(0);
        world.raise_cached_tag_mission_event(&data, &mut state, 0, context(59));
        assert_eq!(state.triggers.iter().find(|trigger| trigger.id == "overflown").unwrap().fire_count, 1);
    }

    #[test]
    fn airborne_source_matches_original_retail_height_frame_placement_and_cell_tag_matrix() {
        let mut cases = 0;
        for row in include_str!("../tests/data/entered_overflown_source_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let value: Vec<i32> = row.split('\t').map(|cell| cell.parse().unwrap()).collect();
            let mut world = field();
            let unit = world.add_unit((5, 6), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
            world.units[0].falling = Some(value[2]);
            if value[1] == 0 {
                let passenger = world.units.pop().unwrap();
                world.add_unit((8, 8), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
                world.units[0].cargo.push(passenger);
            }
            let mut data = mission(vec![trigger("overflown", 59, -1, vec![])]);
            data.yuri = true;
            data.tags[0].persistence = 2;
            if value[3] != 0 { data.cell_tags.push(((5, 6), "tag-overflown".into())); }
            world.attach_mission(data).unwrap();
            world.tick = value[0] as u64;
            assert_eq!(world.register_mission_airborne_event(unit), value[4] != 0, "{row}");
            let count = world.mission_state.as_ref().unwrap().triggers.iter().find(|trigger| trigger.id == "overflown").map_or(0, |trigger| trigger.fire_count);
            assert_eq!(count, value[4] as u64, "{row}");
            cases += 1;
        }
        assert_eq!(cases, 100);
    }

    #[test]
    fn hovering_source_repeats_without_cell_changes_uses_relative_height_and_restores() {
        fn setup(level: u8, bridge: bool, height: i32) -> World {
            let mut world = field();
            world.map.set_cell(5, 6, true, level);
            if bridge { world.map.set_bridge(5, 6, 1); }
            world.add_unit((5, 6), 0, crate::UnitStats { strength: 100, jet: Some(crate::flight::JetStats::default()), ..crate::UnitStats::default() }, 1);
            let jet = world.units[0].jet.as_mut().unwrap();
            jet.z = level as i32 * crate::combat::LEVEL_HEIGHT + height;
            jet.state = crate::flight::HOVERING;
            world.units[0].on_bridge = bridge;
            let mut data = mission(vec![trigger("overflown", 59, -1, vec![])]);
            data.yuri = true;
            data.tags[0].persistence = 2;
            data.cell_tags.push(((5, 6), "tag-overflown".into()));
            world.attach_mission(data).unwrap();
            world
        }
        for level in [0, 3] { for bridge in [false, true] { for height in [0, 207, 208, 500] {
            let mut world = setup(level, bridge, height);
            for _ in 0..15 { world.tick(); }
            assert_eq!(fired(&world, "overflown"), 0);
            let saved = world.snapshot();
            let mut restored = setup(level, bridge, height);
            restored.restore(&saved).unwrap();
            for _ in 0..17 { world.tick(); restored.tick(); }
            assert_eq!(world.state_hash(), restored.state_hash());
            assert_eq!(fired(&world, "overflown"), if height >= 208 { 2 } else { 0 }, "level={level}, bridge={bridge}, height={height}");
            assert_eq!(world.units[0].cell(), (5, 6));
        }}}
    }

    #[test]
    fn airborne_source_reports_unbound_terrain_mark_and_coordinates_instead_of_guessing() {
        for feature in ["AirborneFloorHeight", "AirborneMapBoundary", "AirborneBridgeLayer", "AirborneOnMapState", "AirborneSpecialCoordinates"] {
            let mut world = field();
            let unit = world.add_unit((5, 6), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
            world.units[0].falling = Some(500);
            match feature {
                "AirborneFloorHeight" => world.map.set_ground(5, 6, 1, false),
                "AirborneMapBoundary" => world.units[0].x = -128,
                "AirborneBridgeLayer" => world.units[0].on_bridge = true,
                "AirborneOnMapState" => world.units[0].warp_until = 17,
                "AirborneSpecialCoordinates" => world.units[0].stats.rocket = true,
                _ => unreachable!(),
            }
            let mut data = mission(vec![trigger("overflown", 59, -1, vec![])]);
            data.yuri = true;
            data.tags[0].persistence = 2;
            data.cell_tags.push((world.units[0].cell(), "tag-overflown".into()));
            world.attach_mission(data).unwrap();
            world.tick = 16;
            assert!(!world.register_mission_airborne_event(unit));
            assert_eq!(fired(&world, "overflown"), 0);
            assert!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|item| matches!(&item.kind, DiagnosticKind::CellEntryUnverified { feature: recorded } if recorded == feature)), "{feature}");
        }
    }

    #[test]
    fn entered_by_shared_semipersistent_cells_detach_source_refs_and_resume_deterministically() {
        fn setup() -> (World, u32) {
            let mut world = field();
            let unit = world.add_unit((3, 6), 64, crate::UnitStats { strength: 100, speed: 6, ..crate::UnitStats::default() }, 1);
            let mut data = mission(vec![trigger("entered", 1, -1, vec![ActionDef::with_parameter(56, "0")])]);
            data.tags[0].persistence = 1;
            data.cell_tags = vec![((5, 6), "tag-entered".into()), ((8, 6), "tag-entered".into())];
            data.objects.push(ObjectBinding { section: "Units".into(), id: "0".into(), tag: Some("tag-entered".into()), object: crate::combat::Aim::Unit(unit), group: -1, initial_mission: "Guard".into() });
            world.attach_mission(data).unwrap();
            (world, unit)
        }
        let (mut world, unit) = setup();
        assert_eq!(world.mission_state.as_ref().unwrap().tag_refs(0), 3);
        move_until_still(&mut world, unit, (5, 6));
        let state = world.mission_state.as_ref().unwrap();
        assert_eq!(fired(&world, "entered"), 0);
        assert_eq!(state.tag_refs(0), 1, "owner and entering cell each detach after a non-last p1 match");
        assert_eq!(state.object_tag(crate::combat::Aim::Unit(unit)), None);
        assert_eq!(state.tags[0].cells, [(8, 6)]);
        world.order(crate::Order::Move { unit, x: 8, y: 6 });
        for _ in 0..9 { world.tick(); }
        let bytes = world.snapshot();
        let (mut restored, _) = setup();
        restored.restore(&bytes).unwrap();
        for _ in 0..100 { world.tick(); restored.tick(); }
        assert_eq!(world.snapshot(), restored.snapshot());
        assert_eq!(fired(&world, "entered"), 1);
        let state = world.mission_state.as_ref().unwrap();
        assert!(state.local(0));
        assert_eq!(state.tag_refs(0), 0);
        assert!(state.tags[0].destroyed);
    }

    #[test]
    fn initial_placement_and_blocked_moves_do_not_raise_entered_by() {
        let mut world = field();
        let stationary = world.add_unit((5, 5), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
        let blocked = world.add_unit((3, 8), 64, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
        world.map.set_cell(5, 8, false, 0);
        let mut data = mission(vec![trigger("entered", 1, -1, Vec::new())]);
        data.tags[0].persistence = 2;
        data.cell_tags = vec![((5, 5), "tag-entered".into()), ((5, 8), "tag-entered".into())];
        world.attach_mission(data).unwrap();
        let placed_later = world.add_unit((5, 5), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
        world.order(crate::Order::Move { unit: blocked, x: 5, y: 8 });
        for _ in 0..100 { world.tick(); }
        assert_eq!(world.unit(stationary).unwrap().cell(), (5, 5));
        assert_eq!(world.unit(placed_later).unwrap().cell(), (5, 5));
        assert_ne!(world.unit(blocked).unwrap().cell(), (5, 8));
        assert_eq!(fired(&world, "entered"), 0);
    }

    #[test]
    fn entered_by_actions_run_before_the_same_drives_next_cell_completion() {
        let mut world = field();
        let unit = world.add_unit((3, 6), 64, crate::UnitStats { strength: 100, speed: 255, accelerates: false, ..crate::UnitStats::default() }, 1);
        let first = trigger("first", 1, -1, vec![ActionDef::with_parameter(56, "0")]);
        let mut second = trigger("second", 1, -1, vec![ActionDef::with_parameter(11, "already-set")]);
        second.events.push(EventDef { kind: 36, parameter: 0, ..EventDef::default() });
        let mut data = mission(vec![first, second]);
        for tag in &mut data.tags { tag.persistence = 2; }
        data.cell_tags = vec![((5, 6), "tag-first".into()), ((6, 6), "tag-second".into())];
        world.attach_mission(data).unwrap();
        world.order(crate::Order::Move { unit, x: 9, y: 6 });
        for _ in 0..20 { world.tick(); if fired(&world, "second") > 0 { break; } }
        let entries: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::TriggerFired { tick, trigger_id, .. } = output { Some((tick, trigger_id)) } else { None }).collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].1, "first");
        assert_eq!(entries[1].1, "second");
        assert_eq!(entries[0].0, entries[1].0, "accumulated movement must continue after synchronously executing first cell's actions");
    }

    #[test]
    fn yr_entered_event_house_is_shared_but_each_runtime_trigger_house_is_separate() {
        let mut world = field();
        world.houses.push(House { human: true, ..House::default() });
        let a = world.add_unit((3, 5), 64, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
        let b = world.add_unit((3, 9), 64, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 2);
        let mut definition = trigger("entered", 1, -1, Vec::new());
        definition.events.push(EventDef { kind: 36, parameter: 0, ..EventDef::default() }); // Whole AND stays false.
        let mut data = mission(vec![definition]);
        data.yuri = true;
        data.tags[0].persistence = 2;
        data.tags.push(TagDef { id: "second-instance".into(), trigger: "entered".into(), persistence: 2, ..TagDef::default() });
        data.cell_tags = vec![((5, 5), "tag-entered".into()), ((5, 9), "second-instance".into())];
        world.attach_mission(data).unwrap();
        move_until_still(&mut world, a, (5, 5));
        move_until_still(&mut world, b, (5, 9));
        let state = world.mission_state.as_ref().unwrap();
        assert_eq!(fired(&world, "entered"), 0);
        assert_eq!(state.event_houses.len(), 1, "both Tags share the TriggerType's actual TEvent definition");
        assert_eq!(state.event_houses[0].1[0], Some(2));
        assert_eq!(state.triggers.iter().map(|trigger| trigger.house).collect::<Vec<_>>(), [Some(1), Some(2)]);
    }

    #[test]
    fn teleport_and_falling_completion_raise_once_but_placement_and_warp_wait_do_not() {
        for teleport in [false, true] {
            let mut world = field();
            let unit = if teleport {
                world.add_unit((3, 6), 64, crate::UnitStats { teleporter: true, strength: 100, ..crate::UnitStats::default() }, 1)
            } else {
                world.add_infantry((6, 6), 2, 0, crate::UnitStats { infantry: true, strength: 100, ..crate::UnitStats::default() }, 1).unwrap()
            };
            let mut data = mission(vec![trigger("entered", 1, -1, Vec::new())]);
            data.tags[0].persistence = 2;
            data.cell_tags = vec![((6, 6), "tag-entered".into())];
            world.attach_mission(data).unwrap();
            if teleport {
                world.teleport(0, (6, 6));
                assert_eq!(fired(&world, "entered"), 1);
            } else {
                world.units[0].falling = Some(3);
                world.units[0].dropped_at = world.current_tick();
                world.tick();
                assert_eq!(fired(&world, "entered"), 0);
            }
            for _ in 0..25 { world.tick(); }
            assert_eq!(world.unit(unit).unwrap().cell(), (6, 6));
            assert_eq!(fired(&world, "entered"), 1);
        }
    }

    #[test]
    fn fully_cloaked_foot_skips_cell_tags_in_ra2_but_not_yuri() {
        for yuri in [false, true] {
            let mut world = field();
            let unit = world.add_unit((3, 6), 64, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
            world.units[0].cloak.state = 2;
            let mut data = mission(vec![trigger("entered", 1, -1, Vec::new())]);
            data.yuri = yuri;
            data.tags[0].persistence = 2;
            data.cell_tags = vec![((5, 6), "tag-entered".into())];
            world.attach_mission(data).unwrap();
            move_until_still(&mut world, unit, (5, 6));
            assert_eq!(fired(&world, "entered"), u64::from(yuri));
            assert_eq!(world.unit(unit).unwrap().cloak.state, 2);
        }
    }

    #[test]
    fn accepted_capture_raises_the_building_tag_twice_before_consumption_and_skips_cell_tag() {
        for yuri in [false, true] { for persistence in [0, 2] {
            let mut world = field();
            world.houses[1].country = 13;
            world.building_types.push(crate::BuildingType { strength: 100, size: (1, 1), fighter: crate::combat::Fighter { capturable: true, ..crate::combat::Fighter::default() }, ..crate::BuildingType::default() });
            let building = world.add_map_building(0, 0, (6, 6), 256);
            let engineer = world.add_infantry((3, 6), 2, 64, crate::UnitStats { infantry: true, strength: 100, speed: 6, fighter: crate::combat::Fighter { engineer: true, ..crate::combat::Fighter::default() }, ..crate::UnitStats::default() }, 1).unwrap();
            let mut data = mission(vec![trigger("building", 1, 13, Vec::new()), trigger("cell", 1, -1, Vec::new()), trigger("consumed", 48, 0, Vec::new())]);
            data.yuri = yuri;
            data.tags[0].persistence = persistence;
            data.tags[1].persistence = 2;
            data.objects = vec![
                ObjectBinding { section: "Structures".into(), id: "0".into(), tag: Some("tag-building".into()), object: crate::combat::Aim::Building(building), group: -1, initial_mission: "Guard".into() },
                ObjectBinding { section: "Infantry".into(), id: "0".into(), tag: Some("tag-consumed".into()), object: crate::combat::Aim::Unit(engineer), group: -1, initial_mission: "Guard".into() },
            ];
            data.cell_tags = vec![((6, 6), "tag-cell".into())];
            world.attach_mission(data).unwrap();
            world.order(crate::Order::Enter { unit: engineer, building });
            for _ in 0..300 { world.tick(); if world.unit(engineer).is_none() { break; } }
            assert!(world.unit(engineer).is_none());
            assert_eq!(world.buildings[0].owner, 1);
            assert_eq!(fired(&world, "building"), if persistence == 2 { 2 } else { 1 });
            assert_eq!(fired(&world, "cell"), 0, "accepted Infantry entry returns before Foot's ordinary CellTag source");
            assert_eq!(fired(&world, "consumed"), 1);
            let order: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::TriggerFired { trigger_id, .. } = output { Some(trigger_id) } else { None }).collect();
            assert_eq!(order.last().map(String::as_str), Some("consumed"));
        }}
    }

    #[test]
    fn accepted_utility_building_sources_preserve_entrant_and_cell_tag_lifetimes() {
        for path in ["grind-infantry", "grind-unit", "absorb-infantry", "absorb-unit", "treat", "occupy", "full-repair", "sabotage"] {
            let mut world = field();
            let infantry = !path.ends_with("-unit");
            world.unit_types.push(crate::UnitType { size: 1, cost: 100, ..crate::UnitType::default() });
            world.building_types.push(crate::BuildingType {
                strength: 100, size: (1, 1), grinding: path.starts_with("grind"),
                infantry_absorb: path == "absorb-infantry", unit_absorb: path == "absorb-unit", extra_power: 100, passengers: 3, size_limit: 1,
                hospital: path == "treat", ammo: -1, can_be_occupied: path == "occupy", max_occupants: 3, can_c4: path == "sabotage",
                ..crate::BuildingType::default()
            });
            let building = world.add_map_building(1, 0, (6, 6), 256);
            let unit = world.add_unit((6, 6), 0, crate::UnitStats { kind: 1, infantry, strength: 100,
                fighter: crate::combat::Fighter { engineer: path == "full-repair", c4: path == "sabotage", occupier: path == "occupy", ..crate::combat::Fighter::default() }, ..crate::UnitStats::default() }, 1);
            world.units[0].capture = Some(building);
            if path == "treat" { world.units[0].health = 50; }
            let mut data = mission(vec![trigger("building", 1, -1, Vec::new()), trigger("cell", 1, -1, Vec::new()), trigger("consumed", 48, 0, Vec::new())]);
            data.yuri = true;
            data.tags[0].persistence = 2;
            data.tags[1].persistence = 2;
            data.objects = vec![
                ObjectBinding { section: "Structures".into(), id: "0".into(), tag: Some("tag-building".into()), object: crate::combat::Aim::Building(building), group: -1, initial_mission: "Guard".into() },
                ObjectBinding { section: "Infantry".into(), id: "0".into(), tag: Some("tag-consumed".into()), object: crate::combat::Aim::Unit(unit), group: -1, initial_mission: "Guard".into() },
            ];
            data.cell_tags = vec![((6, 6), "tag-cell".into())];
            world.attach_mission(data).unwrap();
            world.register_mission_cell_entry(unit);
            world.enter_buildings();
            assert_eq!(fired(&world, "building"), u64::from(path != "absorb-unit"), "{path}");
            assert_eq!(fired(&world, "cell"), 0, "{path}");
            assert_eq!(fired(&world, "consumed"), u64::from(path.starts_with("grind")), "{path}");
            assert_eq!(world.mission_state.as_ref().unwrap().tags[1].cells, [(6, 6)], "EmptyCell building source cannot detach unrelated CellTag");
            if matches!(path, "full-repair" | "sabotage") { assert!(world.unit(unit).is_some()); }
        }
    }

    #[test]
    fn building_entry_actions_precede_the_next_foot_in_the_same_movement_pass() {
        let mut world = field();
        world.building_types.push(crate::BuildingType { strength: 100, size: (1, 1), fighter: crate::combat::Fighter { capturable: true, ..crate::combat::Fighter::default() }, ..crate::BuildingType::default() });
        let building = world.add_map_building(0, 0, (6, 6), 256);
        let stats = crate::UnitStats { infantry: true, strength: 100, speed: 6, ..crate::UnitStats::default() };
        let engineer = world.add_infantry((3, 6), 2, 64, crate::UnitStats { fighter: crate::combat::Fighter { engineer: true, ..crate::combat::Fighter::default() }, ..stats }, 1).unwrap();
        let observer = world.add_infantry((3, 10), 2, 64, stats, 1).unwrap();
        let mut later = trigger("later", 1, -1, Vec::new());
        later.enabled = false;
        let mut data = mission(vec![trigger("building", 1, -1, vec![ActionDef::with_parameter(53, "later")]), later]);
        data.yuri = true;
        data.objects.push(ObjectBinding { section: "Structures".into(), id: "0".into(), tag: Some("tag-building".into()), object: crate::combat::Aim::Building(building), group: -1, initial_mission: "Guard".into() });
        data.cell_tags = vec![((6, 10), "tag-later".into())];
        world.attach_mission(data).unwrap();
        world.order(crate::Order::Enter { unit: engineer, building });
        world.order(crate::Order::Move { unit: observer, x: 6, y: 10 });
        for _ in 0..200 { world.tick(); if fired(&world, "later") > 0 { break; } }
        assert!(world.unit(engineer).is_none(), "consumed Infantry must already be unregistered before the later Foot moves");
        let order: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::TriggerFired { tick, trigger_id, .. } = output { Some((tick, trigger_id)) } else { None }).collect();
        assert_eq!(order.len(), 2);
        assert_eq!((&*order[0].1, &*order[1].1), ("building", "later"));
        assert_eq!(order[0].0, order[1].0, "the building action cannot be deferred until the later combat phase");
    }

    #[test]
    fn campaign_shared_sight_is_directional_and_radar_masks_use_country_indices() {
        for (ally_reveal, country_mask) in [(true, true), (false, true), (true, false)] {
            let mut map = Map::new(40, 16);
            for y in 0..16 { for x in 0..40 { map.set_cell(x, y, true, 0); } }
            map.shroud_on = true;
            map.viewer = 1;
            let mut world = World::new(map, None);
            world.houses = vec![House::default(); 5];
            world.houses[1].human = true;
            world.houses[1].country = 7;
            world.houses[1].allies = 1 << 0; // Likes Neutral; Neutral does not like it back.
            world.houses[2].allies = 1 << 1;
            world.houses[3].human = true; // Controlled, but not a sight-sharing ally.
            let stats = crate::UnitStats { strength: 100, fighter: crate::combat::Fighter { sight: 1, ..crate::combat::Fighter::default() }, ..crate::UnitStats::default() };
            for (owner, x) in [(1, 2), (2, 8), (3, 14), (0, 20), (4, 26)] { world.add_unit((x, 6), 0, stats, owner); }
            let mut data = mission(Vec::new());
            data.ally_reveal = ally_reveal;
            data.radar_visible_to = vec![(4, 1 << if country_mask { 7 } else { 1 })];
            world.attach_mission(data).unwrap();
            world.look();
            assert!(world.map.revealed_to(1, 2, 6));
            assert_eq!(world.map.revealed_to(1, 8, 6), ally_reveal);
            assert!(!world.map.revealed_to(1, 14, 6), "PlayerControl alone grants no shared sight");
            assert!(!world.map.revealed_to(1, 20, 6), "source owner's Allies decides the direction");
            assert_eq!(world.map.revealed_to(1, 26, 6), country_mask, "RadarVisibleTo uses Country7, not viewing House1");
            assert!(!world.map.revealed_to(3, 14, 6), "campaign reveal routines write the single CurrentPlayer shroud");
        }
    }

    #[test]
    fn reveal_zone_uses_crusher_connectivity_at_tick_15_and_restores_midway() {
        fn setup(action: i32) -> World {
            let mut map = Map::new(48, 16);
            for y in 0..16 { for x in 0..48 { map.set_cell(x, y, true, 0); } }
            for y in 0..16 {
                map.set_obstacle(14, y, crate::Obstacle::Crushable);
                for x in 32..37 { map.set_land(x, y, crate::Land::Water); }
            }
            map.shroud_on = true;
            map.viewer = 1;
            let mut world = World::new(map, None);
            world.houses = vec![House::default(), House { human: true, ..House::default() }, House { human: true, ..House::default() }];
            let mut data = mission(vec![trigger("uncover", 13, 1, vec![ActionDef::with_parameter(action, "495")])]);
            data.houses.push(HouseBinding { name: "Other".into(), country: "Other".into(), house: 2 });
            data.waypoints = vec![(495, (4, 7))];
            data.reveal_radius = 2;
            world.attach_mission(data).unwrap();
            world
        }
        for action in [17, 18] {
            let mut world = setup(action);
            assert_ne!(world.map.zone_at((4, 7), 0), world.map.zone_at((26, 7), 0));
            assert_eq!(world.map.zone_at((4, 7), 1), world.map.zone_at((26, 7), 1));
            assert_ne!(world.map.zone_at((4, 7), 1), world.map.zone_at((44, 7), 1));
            for _ in 0..14 { world.tick(); }
            assert!(!world.map.revealed(4, 7));
            let saved = world.snapshot();
            let mut restored = setup(action);
            restored.restore(&saved).unwrap();
            for candidate in [&mut world, &mut restored] {
                candidate.tick();
                assert!(candidate.map.revealed(4, 7));
                assert_eq!(candidate.map.revealed(26, 7), action == 18, "only18 reaches distant cells in the same Crusher zone");
                assert!(!candidate.map.revealed(44, 7), "the disconnected far shore stays shrouded");
                assert!(!candidate.map.revealed_to(2, 4, 7), "17/18 target CurrentPlayer, not all Houses");
                assert_eq!(fired(candidate, "uncover"), 1);
            }
            assert_eq!(world.state_hashes(), restored.state_hashes());
        }
    }

    #[test]
    fn reveal_zone_compares_reserved_zone_ids_but_does_not_iterate_outside_usable_area() {
        let mut world = field();
        world.map.shroud_on = true;
        world.map.viewer = 1;
        world.map.set_land(4, 4, crate::Land::Water);
        world.map.set_land(12, 12, crate::Land::Water);
        world.map.build_zones();
        let mut data = mission(vec![trigger("blocked-zone", 8, 0, vec![ActionDef::with_parameter(18, "0")])]);
        data.waypoints = vec![(0, (4, 4))];
        world.attach_mission(data).unwrap();
        assert_eq!(world.map.zone_at((4, 4), 1), 1);
        world.run_mission();
        assert!(world.map.revealed(12, 12), "native equality does not discard reserved blocked zone1");

        let mut world = field();
        world.map.shroud_on = true;
        world.map.set_playfield(8, 8, [1, 1, 6, 6]);
        world.map.build_zones();
        let mut data = mission(vec![trigger("outside-seed", 8, 0, vec![ActionDef::with_parameter(18, "0")])]);
        data.waypoints = vec![(0, (3, 3))];
        world.attach_mission(data).unwrap();
        assert_eq!(world.map.zone_at((3, 3), 1), 0xffff);
        world.run_mission();
        assert!(!world.map.revealed_to(1, 3, 3), "outside cells do not become candidates merely because their zone IDs match");
        assert!(world.map.revealed.iter().all(|mask| *mask == 0));
    }

    #[test]
    fn reveal_actions_keep_the_native_caller_and_callee_height_projection() {
        for action in [17, 18] {
            let mut world = field();
            world.map.shroud_on = true;
            world.map.viewer = 1;
            world.map.set_cell(12, 12, true, 4);
            // Isolate18's zone1 seed so other cells do not reveal over the
            // center being checked. Both positions were independently measured
            // by the original RevealArea2-prefix CPU harness.
            world.map.set_land(12, 12, crate::Land::Water);
            world.map.build_zones();
            let mut data = mission(vec![trigger("reveal", 8, 0, vec![ActionDef::with_parameter(action, "0")])]);
            data.waypoints = vec![(0, (12, 12))];
            data.reveal_radius = 1;
            world.attach_mission(data).unwrap();
            world.run_mission();
            if action == 17 {
                assert!(world.map.revealed(8, 8), "17:12-2*trunc(4/2)=8");
                assert!(!world.map.revealed(10, 10), "a single projection would reveal the wrong center");
            } else {
                assert!(world.map.revealed(8, 10), "18:center10 with radius2 reaches(8,10)");
                assert!(!world.map.revealed(12, 13), "18:12-2*trunc(4/4)=10, not11");
            }
        }
    }

    #[test]
    fn reveal_by_height_can_disable_occlusion_without_changing_the_projection() {
        for by_height in [false, true] {
            let mut world = field();
            world.map.shroud_on = true;
            world.map.viewer = 1;
            // Native occlusion looks at source+2+(spread's step toward source).
            world.map.set_cell(6, 6, true, 7);
            let mut data = mission(vec![trigger("reveal", 8, 0, vec![ActionDef::with_parameter(17, "0")])]);
            data.waypoints = vec![(0, (4, 4))];
            data.reveal_by_height = by_height;
            data.reveal_radius = 1;
            world.attach_mission(data).unwrap();
            world.run_mission();
            assert_eq!(world.map.revealed(4, 4), !by_height);
        }
    }

    fn waypoint_damage_field(yuri: bool, health: i32) -> World {
        let mut world = field();
        world.building_types.push(crate::BuildingType { size: (1, 1), strength: 1000, ..crate::BuildingType::default() });
        world.add_building(1, 0, (4, 4));
        world.buildings[0].health = health;
        // A different first warhead catches accidental use of index0 instead
        // of the C4Warhead rules binding. The ordinary blast uses stock Super's
        // zero spread and 100% verses without bypassing ReceiveDamage.
        world.warheads.push(crate::combat::Warhead { verses: [0.0; 11], ..crate::combat::Warhead::default() });
        world.warheads.push(crate::combat::Warhead::default());
        world.ability_rules.c4_warhead = Some(1);
        let mut data = mission(vec![trigger("damage", 8, 0, vec![ActionDef::with_parameter(63, "364suffix"), ActionDef::with_parameter(11, "outer")])]);
        data.yuri = yuri;
        data.waypoints = vec![(364, (4, 4))];
        world.attach_mission(data).unwrap();
        world
    }

    #[test]
    fn action63_uses_five_c4_area_hits_instead_of_a_single_direct_health_change() {
        for yuri in [false, true] {
            let mut world = waypoint_damage_field(yuri, 769);
            let unit = world.add_unit((4, 4), 0, crate::UnitStats { strength: 1000, ..crate::UnitStats::default() }, 1);
            let untouched = world.add_unit((6, 4), 0, crate::UnitStats { strength: 1000, ..crate::UnitStats::default() }, 1);
            let random = (world.random.clone(), world.campaign_random.clone());
            world.run_mission();
            assert_eq!(world.buildings[0].health, 269, "five zero-distance building hits, as original YR All01UMD's reactor");
            assert_eq!(world.unit(unit).unwrap().health, 900, "the zero-spread C4 warhead reaches this Foot only at its exact center, not the four offset points");
            assert_eq!(world.unit(untouched).unwrap().health, 1000);
            assert_eq!((world.random.clone(), world.campaign_random.clone()), random);
            assert_eq!(fired(&world, "damage"), 1);
            assert!(!world.mission_state.as_ref().unwrap().diagnostics.iter().any(|entry| matches!(entry.kind, DiagnosticKind::UnsupportedAction { kind: 63, .. })));
        }
    }

    #[test]
    fn action63_blast_coordinates_and_order_match_the_original_handler_interface_fixture() {
        // Whole original gamemd.exe 3e81a617..00 Action63 at6e0490,
        // native-light/action63-interface-native-verification.json fixture0.
        // That oracle captures DamageArea's interface; it does not substitute
        // a claim about original DamageArea's full object/damage internals.
        let native_points = [(14208, 19072, 416), (14293, 19157, 416), (14123, 19157, 416), (14293, 18987, 416), (14123, 18987, 416)];
        let mut seed = waypoint_damage_field(true, 769);
        let mut data = seed.mission_data.take().unwrap();
        let mut map = Map::new(128, 128);
        map.set_cell(55, 74, true, 4);
        let mut world = World::new(map, None);
        world.houses = seed.houses;
        world.warheads = seed.warheads;
        world.ability_rules = seed.ability_rules;
        data.triggers[0].actions.truncate(1);
        data.waypoints[0].1 = (55, 74);
        for (n, &(x, y, z)) in native_points.iter().enumerate() {
            assert_eq!(z, world.ground_z(x, y));
            let unit = world.add_unit((55, 74), 0, crate::UnitStats { strength: 100, ..crate::UnitStats::default() }, 1);
            let foot = world.units.iter_mut().find(|foot| foot.id == unit).unwrap();
            foot.x = x;
            foot.y = y;
            let id = format!("point{n}");
            data.triggers.push(trigger(&id, 48, 0, vec![ActionDef::with_parameter(11, id.clone())]));
            data.tags.push(TagDef { id: id.clone(), trigger: id.clone(), ..TagDef::default() });
            data.objects.push(ObjectBinding { section: "Units".into(), id: id.clone(), tag: Some(id), object: crate::combat::Aim::Unit(unit), group: -1, initial_mission: "Guard".into() });
        }
        world.attach_mission(data).unwrap();
        world.run_mission();
        let texts: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
        assert_eq!(texts, ["point0", "point1", "point2", "point3", "point4"]);
    }

    #[test]
    fn action63_respects_armour_and_iron_curtain_and_diagnoses_a_missing_c4_binding() {
        for (verses, curtain, c4, expected) in [(0.5, false, Some(1), 519), (1.0, true, Some(1), 769), (1.0, false, None, 769)] {
            let mut world = waypoint_damage_field(true, 769);
            world.warheads[1].verses = [verses; 11];
            world.buildings[0].curtain_until = if curtain { 100 } else { 0 };
            world.ability_rules.c4_warhead = c4;
            world.run_mission();
            assert_eq!(world.buildings[0].health, expected);
            assert_eq!(world.mission_state.as_ref().unwrap().diagnostics.iter().any(|entry| matches!(entry.kind, DiagnosticKind::InvalidParameter { kind: 63, .. })), c4.is_none());
        }
    }

    #[test]
    fn action63_gets_the_bridge_deck_height_and_its_tail_bypasses_weapon_bridge_gates() {
        for high in [false, true] {
            let mut world = waypoint_damage_field(true, 769);
            world.mission_data.as_mut().unwrap().waypoints[0].1 = (8, 8);
            world.bridges_destroyable = false;
            assert!(!world.warheads[1].wall);
            if high {
                world.map.set_bridge_overlay(8, 8, 24, 0, crate::Land::Water);
                for y in 6..=9 { world.map.set_bridge(8, y, 3); }
            } else {
                for y in 7..=9 { world.map.set_bridge_overlay(8, y, 0x4a, 0, crate::Land::Water); }
            }
            let random = (world.random.clone(), world.campaign_random.clone());
            world.run_mission();
            assert_eq!((world.random.clone(), world.campaign_random.clone()), random, "the sure-hit tail does not draw BridgeStrength rolls");
            assert_eq!(world.take_events().iter().filter(|event| matches!(event, crate::combat::Event::BridgeBroken { .. })).count(), 1);
            if high { assert!(!world.map.bridged(8, 8)); }
            else { assert_eq!(world.map.bridge_kind[world.map.index(8, 8).unwrap()], 0x64); }
        }
        let mut world = waypoint_damage_field(true, 769);
        world.map.set_bridge(4, 4, 3);
        let ground = world.add_unit((4, 4), 0, crate::UnitStats { strength: 1000, ..crate::UnitStats::default() }, 1);
        let deck = world.add_unit((4, 4), 0, crate::UnitStats { strength: 1000, ..crate::UnitStats::default() }, 1);
        world.units.iter_mut().find(|unit| unit.id == deck).unwrap().on_bridge = true;
        world.run_mission();
        assert_eq!(world.unit(ground).unwrap().health, 1000, "the bridge-deck blast spares the object below");
        assert_eq!(world.unit(deck).unwrap().health, 900);
    }

    #[test]
    fn action63_null_source_destruction_keeps_nested_triggers_and_the_outer_action_chain() {
        let mut world = waypoint_damage_field(true, 250);
        world.add_building(1, 0, (10, 10));
        let victim = crate::combat::Aim::Building(world.buildings[0].id);
        let mut data = world.mission_data.take().unwrap();
        let mut dead = trigger("dead", 48, 0, vec![ActionDef::with_parameter(63, "365"), ActionDef::with_parameter(11, "nested")]);
        dead.linked = Some("attacker".into());
        data.triggers.extend([dead, trigger("attacker", 7, 0, vec![ActionDef::with_parameter(11, "wrong-attacker")])]);
        data.tags.push(TagDef { id: "dead-tag".into(), trigger: "dead".into(), ..TagDef::default() });
        data.objects.push(ObjectBinding { section: "Structures".into(), id: "reactor".into(), tag: Some("dead-tag".into()), object: victim, group: -1, initial_mission: String::new() });
        data.waypoints.push((365, (10, 10)));
        world.attach_mission(data).unwrap();
        world.run_mission();
        assert_eq!(world.buildings[0].health, 0);
        assert_eq!(world.buildings[1].health, 500, "the nested Action63 executes immediately in destruction48");
        assert_eq!((fired(&world, "damage"), fired(&world, "dead"), fired(&world, "attacker")), (1, 1, 0));
        let texts: Vec<_> = world.take_mission_outputs().into_iter().filter_map(|output| if let MissionOutput::Text { label } = output { Some(label) } else { None }).collect();
        assert_eq!(texts, ["nested", "outer"]);
        assert!(world.mission_state.as_ref().unwrap().destructions.contains(&victim));
    }

    #[test]
    fn action63_replays_the_timer_boundary_after_save_and_bounds_unknown_waypoints() {
        let mut world = waypoint_damage_field(true, 769);
        let mut data = world.mission_data.take().unwrap();
        data.triggers[0].events[0] = EventDef { kind: 13, parameter: 1, ..EventDef::default() };
        world.attach_mission(data.clone()).unwrap();
        for _ in 0..14 { world.tick(); }
        assert_eq!(world.buildings[0].health, 769);
        let mut restored = waypoint_damage_field(true, 769);
        restored.attach_mission(data).unwrap();
        restored.restore(&world.snapshot()).unwrap();
        for _ in 0..3 { world.tick(); restored.tick(); }
        assert_eq!(world.buildings[0].health, 269);
        assert_eq!(world.snapshot(), restored.snapshot());
        let mut unknown = waypoint_damage_field(true, 769);
        unknown.mission_data.as_mut().unwrap().waypoints.clear();
        unknown.run_mission();
        assert_eq!(unknown.buildings[0].health, 769);
        assert!(unknown.mission_state.as_ref().unwrap().diagnostics.iter().any(|entry| matches!(entry.kind, DiagnosticKind::MissingWaypoint { .. })));
    }
}
