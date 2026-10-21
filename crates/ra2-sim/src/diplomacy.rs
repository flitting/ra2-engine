//! Campaign diplomacy state and the audited runtime MakeAlly/MakeEnemy core.
//! World occupancy, visibility and initialization-mode effects are separate.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AngerNode {
    /// Registered House identity, mapped to this engine's runtime slot.
    pub house: u8,
    pub score: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseDiplomacy {
    pub house: u8,
    /// Original House.ArrayIndex, including native -1. Runtime slots differ.
    pub enemy_index: i32,
    /// Original registration order, normally excluding self. The helpers do
    /// not insert nodes, reorder ties, or merge duplicate pointer identities.
    pub anger: Vec<AngerNode>,
}

/// Temporary native-field projection. Allies uses original ArrayIndex bits;
/// the World bridge converts its runtime ownership bits in both directions.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HouseView {
    pub house: u8,
    pub array_index: i32,
    pub country_index: i32,
    pub allies: u32,
    pub defeated: bool,
    pub passive: bool,
}

fn view(views: &[HouseView], house: u8) -> &HouseView {
    views.iter().find(|view| view.house == house).expect("validated diplomacy House")
}

fn state(states: &mut [HouseDiplomacy], house: u8) -> &mut HouseDiplomacy {
    states.iter_mut().find(|state| state.house == house).expect("validated diplomacy state")
}

fn bit(index: i32) -> u32 { 1u32.wrapping_shl(index as u32) }

pub(crate) fn allied(views: &[HouseView], receiver: u8, target: u8) -> bool {
    let (receiver, target) = (view(views, receiver), view(views, target));
    receiver.house == target.house || receiver.array_index == target.array_index
        || (target.array_index != -1 && receiver.allies & bit(target.array_index) != 0)
}

pub(crate) fn adjust_anger(states: &mut [HouseDiplomacy], views: &[HouseView], receiver: u8, add: i32, target: u8) {
    let state = state(states, receiver);
    for node in &mut state.anger {
        if node.house == target { node.score = node.score.wrapping_add(add); }
    }
    let mut best = 0;
    state.enemy_index = -1;
    for node in &state.anger {
        if node.score > best && !view(views, node.house).defeated && !allied(views, receiver, node.house) {
            best = node.score;
            state.enemy_index = view(views, node.house).array_index;
        }
    }
}

pub(crate) fn can_ally(views: &[HouseView], receiver: u8, target: u8) -> bool {
    if allied(views, receiver, target) || view(views, receiver).defeated { return false; }
    let active = views.iter().filter(|view| !view.defeated && !view.passive);
    let (mut count, mut already) = (0usize, 0usize);
    for candidate in active {
        count += 1;
        if allied(views, receiver, candidate.house) { already += 1; }
    }
    count != already + 1
}

pub(crate) fn make_enemy(states: &mut [HouseDiplomacy], views: &mut [HouseView], receiver: u8, target: u8) {
    // Native order is observable: the first selection still sees the alliance.
    adjust_anger(states, views, receiver, 1, target);
    if !allied(views, receiver, target) { return; }
    let target_bit = bit(view(views, target).array_index);
    views.iter_mut().find(|view| view.house == receiver).unwrap().allies &= !target_bit;
    if allied(views, target, receiver) {
        let receiver_bit = bit(view(views, receiver).array_index);
        views.iter_mut().find(|view| view.house == target).unwrap().allies &= !receiver_bit;
        // This is target itself, not receiver. Normal ctor nodes exclude self.
        adjust_anger(states, views, target, 1, target);
    }
}

pub(crate) fn make_ally(states: &mut [HouseDiplomacy], views: &mut [HouseView], receiver: u8, target: u8) {
    if !can_ally(views, receiver, target) { return; }
    let target_index = view(views, target).array_index;
    views.iter_mut().find(|view| view.house == receiver).unwrap().allies |= bit(target_index);
    let first = state(states, receiver).anger.iter().find(|node| node.house == target).map(|node| node.score);
    if let Some(score) = first { adjust_anger(states, views, receiver, score.wrapping_neg(), target); }
    if state(states, receiver).enemy_index == target_index { state(states, receiver).enemy_index = -1; }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    House(u8),
    Missing,
    UnboundMultiplayer,
}

/// `trigger` distinguishes a null Trigger from one with a null runtime House.
/// The production campaign has no multiplayer slot mapping; it stays bounded.
pub(crate) fn resolve_house(views: &[HouseView], yuri: bool, raw: i32, trigger: Option<Option<u8>>, multiplayer: Option<&[i32; 8]>) -> Resolution {
    if yuri {
        let Some(runtime_house) = trigger else { return Resolution::Missing; };
        if raw == 8997 { return runtime_house.map_or(Resolution::Missing, Resolution::House); }
        if raw == -1 { return Resolution::Missing; }
        if (4475..=4482).contains(&raw) {
            let Some(mapping) = multiplayer else { return Resolution::UnboundMultiplayer; };
            return usize::try_from(mapping[(raw - 4475) as usize]).ok().and_then(|index| views.get(index))
                .map_or(Resolution::Missing, |view| Resolution::House(view.house));
        }
    }
    views.iter().find(|view| view.country_index == raw).map_or(Resolution::Missing, |view| Resolution::House(view.house))
}

/// Retail atoi's decimal prefix and modulo32 arithmetic for numeric Type0.
pub(crate) fn atoi32(text: &str) -> i32 {
    let mut bytes = text.as_bytes().iter().copied().peekable();
    while bytes.peek().is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)) { bytes.next(); }
    let negative = match bytes.peek() { Some(b'-') => { bytes.next(); true }, Some(b'+') => { bytes.next(); false }, _ => false };
    let mut value = 0u32;
    while let Some(byte @ b'0'..=b'9') = bytes.next() { value = value.wrapping_mul(10).wrapping_add((byte - b'0') as u32); }
    if negative { value.wrapping_neg() as i32 } else { value as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diplomacy_matches_complete_original_execute_and_helpers() {
        let (mut passed, mut bounded_faults) = (0, 0);
        for row in include_str!("../tests/data/diplomacy_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let columns: Vec<_> = row.split('\t').collect();
            let n = |index: usize| columns[index].parse::<i64>().unwrap();
            let array = |index: usize| columns[index].split(',').map(|value| value.parse::<i64>().unwrap()).collect::<Vec<_>>();
            let (yuri, op, raw) = (n(0) != 0, n(2), n(3) as i32);
            let (indices, countries, flags, allies, enemies, counts, nodes, scores) =
                (array(9), array(10), array(11), array(12), array(13), array(14), array(15), array(16));
            let mut views = (0..4).map(|house| HouseView { house: house as u8, array_index: indices[house] as i32,
                country_index: countries[house] as i32, allies: allies[house] as u32,
                defeated: flags[house] & 4 != 0, passive: flags[house] & 8 != 0 }).collect::<Vec<_>>();
            if n(5) != 0 { views.swap(0, 2); }
            let mut states = (0..4).map(|house| HouseDiplomacy { house: house as u8, enemy_index: enemies[house] as i32,
                anger: (0..counts[house].max(0) as usize).map(|index| AngerNode {
                    house: nodes[house * 4 + index] as u8, score: scores[house * 4 + index] as i32 }).collect() }).collect::<Vec<_>>();
            let trigger = match n(4) { 0 => None, 1 => Some(None), 2 => Some(Some(2)), _ => unreachable!() };
            let resolved = resolve_house(&views, yuri, raw, trigger, Some(&[2, -1, 99, 0, 1, 3, 2, 1]));
            if n(17) != 0 {
                assert!(!yuri && matches!(op, 3 | 4) && raw != -1 && resolved == Resolution::Missing, "{}", columns[1]);
                bounded_faults += 1;
                continue;
            }
            let mut result = None;
            match op {
                0 => adjust_anger(&mut states, &views, 0, n(8) as i32, n(7) as u8),
                1 => make_enemy(&mut states, &mut views, 0, n(7) as u8),
                2 => make_ally(&mut states, &mut views, 0, n(7) as u8),
                3 | 4 => {
                    if !yuri && raw == -1 { result = Some(true); }
                    else if let Resolution::House(target) = resolved {
                        if op == 3 {
                            make_ally(&mut states, &mut views, 0, target);
                            make_ally(&mut states, &mut views, target, 0);
                        } else {
                            make_enemy(&mut states, &mut views, 0, target);
                            make_enemy(&mut states, &mut views, target, 0);
                        }
                        result = Some(true);
                    } else { result = Some(false); }
                },
                5 => assert_eq!(match resolved { Resolution::House(house) => house as i64, _ => -1 }, n(22), "{}", columns[1]),
                _ => unreachable!(),
            }
            if let Some(result) = result { assert_eq!(result, n(18) & 0xff != 0, "{}", columns[1]); }
            let (expected_enemies, expected_allies, expected_scores) = (array(19), array(20), array(21));
            for house in 0..4 {
                assert_eq!(states[house].enemy_index, expected_enemies[house] as i32, "{} house{house} enemy", columns[1]);
                assert_eq!(view(&views, house as u8).allies, expected_allies[house] as u32, "{} house{house} allies", columns[1]);
                let mut actual_scores = scores[house * 4..house * 4 + 4].to_vec();
                for (index, node) in states[house].anger.iter().enumerate() { actual_scores[index] = node.score as i64; }
                assert_eq!(actual_scores, expected_scores[house * 4..house * 4 + 4], "{} house{house} nodes", columns[1]);
            }
            passed += 1;
        }
        assert_eq!((passed, bounded_faults), (5848, 36));
    }

    #[test]
    fn numeric_action_values_match_both_original_atoi_implementations() {
        let mut cases = 0;
        for row in include_str!("../tests/data/diplomacy_atoi_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let columns: Vec<_> = row.split('\t').collect();
            let bytes = columns[1].as_bytes().chunks_exact(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(atoi32(&text), columns[2].parse::<i32>().unwrap(), "edition{} {text:?}", columns[0]);
            cases += 1;
        }
        assert_eq!(cases, 44);
    }
}
