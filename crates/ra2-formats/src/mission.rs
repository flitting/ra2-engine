//! The data-driven part of original RA2 and Yuri's Revenge missions.
//!
//! This is a structural parser, not a mission interpreter. Lists retain file
//! order (including duplicate numeric keys); references are kept as identifiers
//! even when the target is absent. Unknown opcodes and uninterpreted properties
//! survive parsing so that the runtime can report unsupported behaviour instead
//! of silently omitting it.

use std::{error::Error, fmt};

use crate::ini::Ini;

pub type Properties = Vec<(String, String)>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MissionDefinition {
    pub basic: BasicDefinition,
    pub houses: Vec<NamedDefinition>,
    pub countries: Vec<NamedDefinition>,
    pub triggers: Vec<TriggerDefinition>,
    pub events: Vec<TriggerEvents>,
    pub actions: Vec<TriggerActions>,
    pub tags: Vec<TagDefinition>,
    pub cell_tags: Vec<CellTagDefinition>,
    pub variables: Vec<VariableDefinition>,
    pub waypoints: Vec<WaypointDefinition>,
    pub task_forces: Vec<TaskForceDefinition>,
    pub scripts: Vec<ScriptDefinition>,
    pub team_types: Vec<TeamTypeDefinition>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BasicDefinition {
    pub player: Option<String>,
    pub name: Option<String>,
    pub next_scenario: Option<String>,
    pub alt_next_scenario: Option<String>,
    pub properties: Properties,
}

/// An entry in `[Houses]` or `[Countries]`, with its named section preserved.
/// `id` is the list key; `name` is the section name, not its `Name=` property.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamedDefinition {
    pub id: String,
    pub name: String,
    pub properties: Properties,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerDefinition {
    pub id: String,
    /// A CountryType identifier, not a House index or house name.
    pub country: String,
    pub linked_trigger: Option<String>,
    pub name: String,
    pub disabled: bool,
    /// Easy, normal and hard, in that order in the original file.
    pub difficulty: [bool; 3],
    pub must_transfer: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerEvents {
    pub trigger_id: String,
    pub events: Vec<EventDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventDefinition {
    pub opcode: i32,
    pub parameter_type: i32,
    /// Kept textual: depending on the opcode this can be an ID or a number.
    pub value: String,
    /// YR events 60 and 61 have a fourth field, even within a mixed event list.
    pub techno_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerActions {
    pub trigger_id: String,
    pub actions: Vec<ActionDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionDefinition {
    pub opcode: i32,
    pub parameter_type: i32,
    pub parameter: String,
    pub arguments: [String; 4],
    /// The eighth serialized field. Some actions use this as a number (for
    /// example an animation frame count), so parsing must not decode it as a
    /// waypoint unconditionally.
    pub waypoint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagRepeat {
    Once,
    SemiPersistent,
    Repeating,
    Unknown(i32),
}

impl TagRepeat {
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Once,
            1 => Self::SemiPersistent,
            2 => Self::Repeating,
            value => Self::Unknown(value),
        }
    }

    pub fn raw_value(self) -> i32 {
        match self {
            Self::Once => 0,
            Self::SemiPersistent => 1,
            Self::Repeating => 2,
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagDefinition {
    pub id: String,
    pub repeat: TagRepeat,
    pub name: String,
    pub trigger_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellTagDefinition {
    pub cell: i32,
    pub x: i32,
    pub y: i32,
    pub tag_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableDefinition {
    pub id: i32,
    pub name: String,
    pub initial_value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaypointDefinition {
    pub id: i32,
    pub cell: i32,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskForceDefinition {
    pub id: String,
    pub name: Option<String>,
    pub group: Option<i32>,
    pub members: Vec<TaskForceMember>,
    pub properties: Properties,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskForceMember {
    pub index: String,
    pub count: i32,
    pub techno_type: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptDefinition {
    pub id: String,
    pub name: Option<String>,
    pub lines: Vec<ScriptLine>,
    pub properties: Properties,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptLine {
    /// Original key. A script need not start at zero, and keys may repeat.
    pub index: String,
    pub opcode: i32,
    pub argument: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamTypeDefinition {
    pub id: String,
    pub name: Option<String>,
    /// Team types normally refer to a country here, despite the field's name.
    pub house: Option<String>,
    pub task_force: Option<String>,
    pub script: Option<String>,
    pub tag: Option<String>,
    /// Kept textual to distinguish missing values from zero and sentinels.
    pub waypoint: Option<String>,
    pub transport_waypoint: Option<String>,
    pub properties: Properties,
}

fn property<'a>(properties: &'a Properties, key: &str) -> Option<&'a str> {
    properties
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}

macro_rules! property_lookup {
    ($($ty:ty),+ $(,)?) => {$(
        impl $ty {
            pub fn get(&self, key: &str) -> Option<&str> {
                property(&self.properties, key)
            }
        }
    )+};
}

property_lookup!(
    BasicDefinition,
    NamedDefinition,
    TaskForceDefinition,
    ScriptDefinition,
    TeamTypeDefinition,
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionParseError {
    pub section: String,
    pub key: String,
    pub message: String,
}

impl MissionParseError {
    fn new(section: &str, key: &str, message: impl Into<String>) -> Self {
        Self {
            section: section.into(),
            key: key.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for MissionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.section, self.key, self.message)
    }
}

impl Error for MissionParseError {}

type ParseResult<T> = Result<T, MissionParseError>;

fn fields(value: &str) -> Vec<&str> {
    value.split(',').map(str::trim).collect()
}

fn require_len(section: &str, key: &str, fields: &[&str], expected: usize) -> ParseResult<()> {
    if fields.len() == expected {
        Ok(())
    } else {
        Err(MissionParseError::new(
            section,
            key,
            format!("expected {expected} fields, got {}", fields.len()),
        ))
    }
}

fn integer(section: &str, key: &str, value: &str) -> ParseResult<i32> {
    value
        .parse()
        .map_err(|_| MissionParseError::new(section, key, format!("invalid integer {value:?}")))
}

fn boolean(section: &str, key: &str, value: &str) -> ParseResult<bool> {
    if ["1", "yes", "true"]
        .iter()
        .any(|x| value.eq_ignore_ascii_case(x))
    {
        Ok(true)
    } else if ["0", "no", "false"]
        .iter()
        .any(|x| value.eq_ignore_ascii_case(x))
    {
        Ok(false)
    } else {
        Err(MissionParseError::new(
            section,
            key,
            format!("invalid boolean {value:?}"),
        ))
    }
}

fn reference(value: &str) -> Option<String> {
    (!value.is_empty() && !value.eq_ignore_ascii_case("<none>")).then(|| value.to_owned())
}

fn optional(ini: &Ini, section: &str, key: &str) -> Option<String> {
    ini.get(section, key).and_then(reference)
}

fn named(ini: &Ini, registry: &str) -> Vec<NamedDefinition> {
    ini.section(registry)
        .iter()
        .map(|(id, name)| NamedDefinition {
            id: id.clone(),
            name: name.clone(),
            properties: ini.section(name).to_vec(),
        })
        .collect()
}

impl MissionDefinition {
    pub fn parse(ini: &Ini) -> ParseResult<Self> {
        let mut mission = Self {
            basic: BasicDefinition {
                player: optional(ini, "Basic", "Player"),
                name: optional(ini, "Basic", "Name"),
                next_scenario: optional(ini, "Basic", "NextScenario"),
                alt_next_scenario: optional(ini, "Basic", "AltNextScenario"),
                properties: ini.section("Basic").to_vec(),
            },
            houses: named(ini, "Houses"),
            countries: named(ini, "Countries"),
            ..Self::default()
        };

        for (id, value) in ini.section("Triggers") {
            let f = fields(value);
            require_len("Triggers", id, &f, 8)?;
            mission.triggers.push(TriggerDefinition {
                id: id.clone(),
                country: f[0].into(),
                linked_trigger: reference(f[1]),
                name: f[2].into(),
                disabled: boolean("Triggers", id, f[3])?,
                difficulty: [
                    boolean("Triggers", id, f[4])?,
                    boolean("Triggers", id, f[5])?,
                    boolean("Triggers", id, f[6])?,
                ],
                must_transfer: boolean("Triggers", id, f[7])?,
            });
        }

        for (id, value) in ini.section("Events") {
            let f = fields(value);
            let count = record_count("Events", id, &f)?;
            let mut at = 1;
            let mut events = Vec::new();
            for _ in 0..count {
                let opcode = integer("Events", id, f.get(at).copied().unwrap_or(""))?;
                let width = if matches!(opcode, 60 | 61) { 4 } else { 3 };
                let r = f.get(at..at + width).ok_or_else(|| {
                    MissionParseError::new("Events", id, "truncated event record")
                })?;
                events.push(EventDefinition {
                    opcode,
                    parameter_type: integer("Events", id, r[1])?,
                    value: r[2].into(),
                    techno_type: (width == 4).then(|| r[3].to_owned()),
                });
                at += width;
            }
            require_len("Events", id, &f, at)?;
            mission.events.push(TriggerEvents {
                trigger_id: id.clone(),
                events,
            });
        }

        for (id, value) in ini.section("Actions") {
            let f = fields(value);
            let count = record_count("Actions", id, &f)?;
            // Check the size before multiplication/allocation on untrusted files.
            if count > f.len().saturating_sub(1) / 8 || f.len() != 1 + count * 8 {
                return Err(MissionParseError::new(
                    "Actions",
                    id,
                    format!(
                        "declares {count} actions but has {} action fields",
                        f.len() - 1,
                    ),
                ));
            }
            let mut actions = Vec::new();
            for r in f[1..].chunks_exact(8) {
                actions.push(ActionDefinition {
                    opcode: integer("Actions", id, r[0])?,
                    parameter_type: integer("Actions", id, r[1])?,
                    parameter: r[2].into(),
                    arguments: std::array::from_fn(|i| r[i + 3].to_owned()),
                    waypoint: r[7].into(),
                });
            }
            mission.actions.push(TriggerActions {
                trigger_id: id.clone(),
                actions,
            });
        }

        for (id, value) in ini.section("Tags") {
            let f = fields(value);
            require_len("Tags", id, &f, 3)?;
            mission.tags.push(TagDefinition {
                id: id.clone(),
                repeat: TagRepeat::from_raw(integer("Tags", id, f[0])?),
                name: f[1].into(),
                trigger_id: f[2].into(),
            });
        }
        for (key, tag_id) in ini.section("CellTags") {
            let cell = integer("CellTags", key, key)?;
            mission.cell_tags.push(CellTagDefinition {
                cell,
                x: cell % 1000,
                y: cell / 1000,
                tag_id: tag_id.clone(),
            });
        }
        for (key, value) in ini.section("VariableNames") {
            let f = fields(value);
            require_len("VariableNames", key, &f, 2)?;
            mission.variables.push(VariableDefinition {
                id: integer("VariableNames", key, key)?,
                name: f[0].into(),
                initial_value: integer("VariableNames", key, f[1])?,
            });
        }
        for (key, value) in ini.section("Waypoints") {
            let cell = integer("Waypoints", key, value)?;
            mission.waypoints.push(WaypointDefinition {
                id: integer("Waypoints", key, key)?,
                cell,
                x: cell % 1000,
                y: cell / 1000,
            });
        }

        for (_, id) in ini.section("TaskForces") {
            let mut members = Vec::new();
            for (key, value) in ini.section(id) {
                if key.parse::<i32>().is_err() {
                    continue;
                }
                let f = fields(value);
                require_len(id, key, &f, 2)?;
                members.push(TaskForceMember {
                    index: key.clone(),
                    count: integer(id, key, f[0])?,
                    techno_type: f[1].into(),
                });
            }
            mission.task_forces.push(TaskForceDefinition {
                id: id.clone(),
                name: optional(ini, id, "Name"),
                group: ini
                    .get(id, "Group")
                    .map(|value| integer(id, "Group", value))
                    .transpose()?,
                members,
                properties: ini.section(id).to_vec(),
            });
        }
        for (_, id) in ini.section("ScriptTypes") {
            let mut lines = Vec::new();
            for (key, value) in ini.section(id) {
                if key.parse::<i32>().is_err() {
                    continue;
                }
                let f = fields(value);
                require_len(id, key, &f, 2)?;
                lines.push(ScriptLine {
                    index: key.clone(),
                    opcode: integer(id, key, f[0])?,
                    argument: f[1].into(),
                });
            }
            mission.scripts.push(ScriptDefinition {
                id: id.clone(),
                name: optional(ini, id, "Name"),
                lines,
                properties: ini.section(id).to_vec(),
            });
        }
        for (_, id) in ini.section("TeamTypes") {
            mission.team_types.push(TeamTypeDefinition {
                id: id.clone(),
                name: optional(ini, id, "Name"),
                house: optional(ini, id, "House"),
                task_force: optional(ini, id, "TaskForce"),
                script: optional(ini, id, "Script"),
                tag: optional(ini, id, "Tag"),
                waypoint: optional(ini, id, "Waypoint"),
                transport_waypoint: optional(ini, id, "TransportWaypoint"),
                properties: ini.section(id).to_vec(),
            });
        }
        Ok(mission)
    }

    pub fn events_for(&self, trigger_id: &str) -> Option<&[EventDefinition]> {
        self.events
            .iter()
            .find(|entry| entry.trigger_id.eq_ignore_ascii_case(trigger_id))
            .map(|entry| entry.events.as_slice())
    }

    pub fn actions_for(&self, trigger_id: &str) -> Option<&[ActionDefinition]> {
        self.actions
            .iter()
            .find(|entry| entry.trigger_id.eq_ignore_ascii_case(trigger_id))
            .map(|entry| entry.actions.as_slice())
    }

    pub fn waypoint(&self, id: i32) -> Option<&WaypointDefinition> {
        self.waypoints.iter().find(|waypoint| waypoint.id == id)
    }
}

fn record_count(section: &str, key: &str, fields: &[&str]) -> ParseResult<usize> {
    fields[0].parse::<usize>().map_err(|_| {
        MissionParseError::new(
            section,
            key,
            format!("invalid record count {:?}", fields[0]),
        )
    })
}

/// Converts a waypoint alphabet code only when the caller knows the field is a
/// waypoint: `A=0`, `Z=25`, `AA=26`. Numeric action arguments are not codes.
pub fn waypoint_index(code: &str) -> Option<i32> {
    if code.is_empty() {
        return None;
    }
    let mut index = 0i32;
    for byte in code.bytes() {
        if !byte.is_ascii_alphabetic() {
            return None;
        }
        index = index
            .checked_mul(26)?
            .checked_add(i32::from(byte.to_ascii_uppercase() - b'A') + 1)?;
    }
    index.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> MissionDefinition {
        MissionDefinition::parse(&Ini::parse(text.as_bytes())).unwrap()
    }

    #[test]
    fn mixed_yr_and_original_events_keep_record_boundaries_and_unknown_opcodes() {
        let mission = parse("[Events]\nT=4,60,2,1,GAWEAP,13,0,3,61,2,0,TANY,999,7,unknown\n");
        let events = mission.events_for("t").unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(
            (
                events[0].opcode,
                events[0].value.as_str(),
                events[0].techno_type.as_deref()
            ),
            (60, "1", Some("GAWEAP"))
        );
        assert_eq!(
            (
                events[1].opcode,
                events[1].value.as_str(),
                events[1].techno_type.as_deref()
            ),
            (13, "3", None)
        );
        assert_eq!(
            (events[2].opcode, events[2].techno_type.as_deref()),
            (61, Some("TANY"))
        );
        assert_eq!(
            (
                events[3].opcode,
                events[3].parameter_type,
                events[3].value.as_str()
            ),
            (999, 7, "unknown")
        );
    }

    #[test]
    fn actions_keep_all_eight_fields_and_do_not_decode_numeric_waypoint_slot() {
        let mission = parse("[Actions]\nT=2,104,5,TEAM,11,22,33,44,90,999,8,OTHER,a,b,c,d,AA\n");
        let actions = mission.actions_for("T").unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].arguments, ["11", "22", "33", "44"]);
        assert_eq!(actions[0].waypoint, "90");
        assert_eq!(actions[1].opcode, 999);
        assert_eq!(actions[1].arguments, ["a", "b", "c", "d"]);
        assert_eq!(actions[1].waypoint, "AA");
        assert_eq!(waypoint_index(&actions[0].waypoint), None);
        assert_eq!(waypoint_index(&actions[1].waypoint), Some(26));
    }

    #[test]
    fn definitions_keep_file_order_properties_duplicate_keys_and_dangling_references() {
        let mission = parse(
            "\
[Basic]
Player=Player House
NextScenario=<none>
CustomProperty=keep
[Houses]
3=Player House
1=Missing House
[Player House]
Country=Player
Credits=40
UnknownFlag=perhaps
[Countries]
8=Player
[Player]
ParentCountry=Americans
[Triggers]
BBBB=Player,MissingTrigger,later,1,1,0,1,1
AAAA=MissingCountry,<none>,first,0,0,1,1,0
[Tags]
TAG=7,unknown repeat,MissingTrigger
[CellTags]
123045=MissingTag
[VariableNames]
9=Objective,1
[Waypoints]
7=123045
0=56012
[TaskForces]
3=TF
[TF]
Name=A force
0=1,E1
0=2,E2
Group=-1
NewFlag=keep
[ScriptTypes]
0=SCRIPT
[SCRIPT]
Name=A script
2=3,7
9=999,raw argument
9=6,1
[TeamTypes]
0=TEAM
[TEAM]
Name=A team
House=Player
TaskForce=MissingForce
Script=SCRIPT
Tag=MissingTag
Waypoint=0
TransportWaypoint=-1
NewFlag=keep
",
        );
        assert_eq!(mission.basic.player.as_deref(), Some("Player House"));
        assert_eq!(mission.basic.next_scenario, None);
        assert_eq!(mission.basic.get("customproperty"), Some("keep"));
        assert_eq!(
            (
                mission.houses[0].id.as_str(),
                mission.houses[0].name.as_str()
            ),
            ("3", "Player House")
        );
        assert_eq!(mission.houses[0].get("UnknownFlag"), Some("perhaps"));
        assert!(mission.houses[1].properties.is_empty());
        assert_eq!(mission.countries[0].get("ParentCountry"), Some("Americans"));
        assert_eq!(
            mission
                .triggers
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            ["BBBB", "AAAA"]
        );
        assert_eq!(
            mission.triggers[0].linked_trigger.as_deref(),
            Some("MissingTrigger")
        );
        assert_eq!(mission.triggers[0].difficulty, [true, false, true]);
        assert!(mission.triggers[0].must_transfer);
        assert_eq!(mission.tags[0].repeat, TagRepeat::Unknown(7));
        assert_eq!(mission.tags[0].repeat.raw_value(), 7);
        assert_eq!((mission.cell_tags[0].x, mission.cell_tags[0].y), (45, 123));
        assert_eq!(
            (mission.variables[0].id, mission.variables[0].initial_value),
            (9, 1)
        );
        assert_eq!(
            mission.waypoints.iter().map(|w| w.id).collect::<Vec<_>>(),
            [7, 0]
        );
        assert_eq!(
            (
                mission.waypoint(7).unwrap().x,
                mission.waypoint(7).unwrap().y
            ),
            (45, 123)
        );
        assert_eq!(
            mission.task_forces[0]
                .members
                .iter()
                .map(|m| (m.index.as_str(), m.count))
                .collect::<Vec<_>>(),
            [("0", 1), ("0", 2)]
        );
        assert_eq!(mission.task_forces[0].get("NewFlag"), Some("keep"));
        assert_eq!(
            mission.scripts[0]
                .lines
                .iter()
                .map(|l| l.index.as_str())
                .collect::<Vec<_>>(),
            ["2", "9", "9"]
        );
        assert_eq!(mission.scripts[0].lines[1].argument, "raw argument");
        assert_eq!(
            mission.team_types[0].task_force.as_deref(),
            Some("MissingForce")
        );
        assert_eq!(
            mission.team_types[0].transport_waypoint.as_deref(),
            Some("-1")
        );
        assert_eq!(mission.team_types[0].get("NewFlag"), Some("keep"));
    }

    #[test]
    fn malformed_lists_report_location_instead_of_losing_operations() {
        for (section, value) in [
            ("Events", "2,13,0,1"),
            ("Events", "1,13,0,1,extra"),
            ("Events", "1,60,2,1"),
            ("Actions", "2,4,1,TEAM,0,0,0,0,A"),
            ("Actions", "1,4,1,TEAM,0,0,0,0,A,extra"),
            ("Triggers", "Player,<none>,Name,2,1,1,1,0"),
            ("Tags", "0,Name"),
        ] {
            let text = format!("[{section}]\nABC={value}\n");
            let error = MissionDefinition::parse(&Ini::parse(text.as_bytes())).unwrap_err();
            assert_eq!(
                (error.section.as_str(), error.key.as_str()),
                (section, "ABC")
            );
            assert!(error.to_string().contains("ABC"));
        }
    }

    #[test]
    fn empty_document_and_zero_record_lists_are_valid() {
        assert_eq!(parse(""), MissionDefinition::default());
        let mission = parse("[Events]\nT=0\n[Actions]\nT=0\n");
        assert_eq!(mission.events_for("T"), Some([].as_slice()));
        assert_eq!(mission.actions_for("T"), Some([].as_slice()));
    }

    #[test]
    fn waypoint_codes_are_checked_case_insensitively() {
        for (code, expected) in [
            ("A", 0),
            ("Z", 25),
            ("AA", 26),
            ("AZ", 51),
            ("BA", 52),
            ("zz", 701),
            ("AAA", 702),
        ] {
            assert_eq!(waypoint_index(code), Some(expected));
        }
        for code in ["", "0", "90", "A1", "<none>", "AAAAAAAAA", "é"] {
            assert_eq!(waypoint_index(code), None);
        }
    }

    /// Uses original files in place; no copyrighted map fixtures are committed.
    /// Run with `cargo test -p ra2-formats steam_campaigns -- --ignored` and set
    /// RA2_STEAM_DIR if the normal checkout asset link is unavailable.
    #[test]
    #[ignore = "requires the user's original RA2 and Yuri's Revenge Steam assets"]
    fn steam_campaigns_all_38_retain_every_operation() {
        use crate::mix::Mix;
        use std::path::{Path, PathBuf};

        let root = std::env::var_os("RA2_STEAM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app_2229850/depot_2229851")
            });
        let files = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        let mut archives = Vec::new();
        // Keep patch precedence: the released YR patch includes ALL02UMD.
        for name in ["expandmd01.mix", "mapsmd03.mix", "maps01.mix", "maps02.mix"] {
            let path = files.iter().find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(name)
            });
            if let Some(path) = path {
                archives.push(Mix::open(path).unwrap());
            }
        }
        const ORIGINAL: [&str; 24] = [
            "all01t.map",
            "all02s.map",
            "all03u.map",
            "all04u.map",
            "all05s.map",
            "all06u.map",
            "all07t.map",
            "all08u.map",
            "all09t.map",
            "all10s.map",
            "all11t.map",
            "all12s.map",
            "sov01t.map",
            "sov02t.map",
            "sov03u.map",
            "sov04s.map",
            "sov05u.map",
            "sov06t.map",
            "sov07s.map",
            "sov08u.map",
            "sov09u.map",
            "sov10t.map",
            "sov11s.map",
            "sov12s.map",
        ];
        const YR: [&str; 14] = [
            "all01umd.map",
            "all02umd.map",
            "all03umd.map",
            "all04dmd.map",
            "all05umd.map",
            "all06umd.map",
            "all07smd.map",
            "sov01umd.map",
            "sov02smd.map",
            "sov03umd.map",
            "sov04dmd.map",
            "sov05umd.map",
            "sov06lmd.map",
            "sov07tmd.map",
        ];
        let mut parsed = 0;
        for name in ORIGINAL.into_iter().chain(YR) {
            let data = archives
                .iter()
                .find_map(|archive| archive.get(name))
                .unwrap_or_else(|| panic!("missing original map {name} in {}", root.display()));
            let ini = Ini::parse(data);
            let mission =
                MissionDefinition::parse(&ini).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                mission.triggers.len(),
                ini.section("Triggers").len(),
                "{name}"
            );
            assert_eq!(mission.tags.len(), ini.section("Tags").len(), "{name}");
            assert_eq!(
                mission.cell_tags.len(),
                ini.section("CellTags").len(),
                "{name}"
            );
            assert_eq!(
                mission.task_forces.len(),
                ini.section("TaskForces").len(),
                "{name}"
            );
            assert_eq!(
                mission.scripts.len(),
                ini.section("ScriptTypes").len(),
                "{name}"
            );
            assert_eq!(
                mission.team_types.len(),
                ini.section("TeamTypes").len(),
                "{name}"
            );
            let event_count: usize = ini
                .section("Events")
                .iter()
                .map(|(_, value)| value.split(',').next().unwrap().parse::<usize>().unwrap())
                .sum();
            let action_count: usize = ini
                .section("Actions")
                .iter()
                .map(|(_, value)| value.split(',').next().unwrap().parse::<usize>().unwrap())
                .sum();
            assert_eq!(
                mission
                    .events
                    .iter()
                    .map(|entry| entry.events.len())
                    .sum::<usize>(),
                event_count,
                "{name}"
            );
            assert_eq!(
                mission
                    .actions
                    .iter()
                    .map(|entry| entry.actions.len())
                    .sum::<usize>(),
                action_count,
                "{name}"
            );
            for force in &mission.task_forces {
                assert_eq!(
                    force.members.len(),
                    ini.section(&force.id)
                        .iter()
                        .filter(|(key, _)| key.parse::<i32>().is_ok())
                        .count(),
                    "{name} force {}",
                    force.id
                );
            }
            for script in &mission.scripts {
                assert_eq!(
                    script.lines.len(),
                    ini.section(&script.id)
                        .iter()
                        .filter(|(key, _)| key.parse::<i32>().is_ok())
                        .count(),
                    "{name} script {}",
                    script.id
                );
            }
            eprintln!(
                "{name}: {} triggers, {event_count} events, {action_count} actions",
                mission.triggers.len()
            );
            parsed += 1;
        }
        assert_eq!(parsed, 38);
    }
}
