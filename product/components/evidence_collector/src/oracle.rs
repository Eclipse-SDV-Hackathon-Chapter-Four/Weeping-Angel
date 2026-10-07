//! Transition oracle (`<prefix>.oracle.yaml`) and its projection onto the
//! fault level published by the Guardian.
//!
//! The oracle lists Guardian detection transitions per signal. The Guardian
//! topic carries fault-level changes: a mapped class/level pair is `Failed`
//! while at least one signal is active and `Passed` once the last one cleared
//! (`FaultAggregator` in the Guardian). Pairs without a DFM fault are not
//! published and therefore not applicable here.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::Stage;

/// Guardian-to-DFM projection: (detection class, level) -> fault id.
/// Copy of `DFM_MAPPINGS` in `guardien/source/guardian_faults.rs` (checked
/// by a test).
pub const DFM_MAPPINGS: [(&str, &str, &str); 12] = [
    ("STREAM_STALE", "VIOLATION", "BatteryTempStreamStale"),
    (
        "STREAM_GENERATION_GAP",
        "VIOLATION",
        "BatteryTempGenerationGap",
    ),
    ("THERMAL_LIMIT", "WARNING", "BatteryOverTempWarning"),
    ("THERMAL_LIMIT", "CRITICAL", "BatteryOverTempCritical"),
    (
        "PHYSICAL_TEMP_ABSOLUTE_LIMIT",
        "VIOLATION",
        "BatteryTempAbsoluteLimit",
    ),
    ("PHYSICAL_TEMP_ORDERING", "VIOLATION", "BatteryTempOrdering"),
    ("PHYSICAL_TEMP_SPREAD", "VIOLATION", "BatteryTempSpread"),
    ("PHYSICAL_TEMP_HOTSPOT", "VIOLATION", "BatteryTempHotspot"),
    ("PHYSICAL_TEMP_RATE", "VIOLATION", "BatteryTempRate"),
    ("PHYSICAL_SOC_RANGE", "VIOLATION", "BatterySocRange"),
    ("PHYSICAL_SOC_RATE", "VIOLATION", "BatterySocRate"),
    ("SIGNAL_STUCK", "VIOLATION", "BatterySignalStuck"),
];

pub fn fault_id(class: &str, level: &str) -> Option<&'static str> {
    DFM_MAPPINGS
        .iter()
        .find(|(c, l, _)| *c == class && *l == level)
        .map(|(_, _, id)| *id)
}

/// One detection transition of the oracle (compact forms expanded).
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub at_ms: u64,
    pub class: String,
    pub level: String,
    pub active: bool,
    pub signal: Option<String>,
}

/// Source window `{ start, end }` in ms (oracle header, case mutator).
#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Window {
    /// Kept for the documented header shape; only `end` is judged here.
    #[allow(dead_code)]
    pub start: u64,
    pub end: u64,
}

pub struct Oracle {
    pub allow_unspecified: bool,
    pub transitions: Vec<Transition>,
    /// `source_window_ms` header of scenario oracles; `None` for hand-written
    /// ones. The end marks the end of the covered stream.
    pub source_window_ms: Option<Window>,
}

/// A fault-level change the Guardian must publish.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Expected {
    pub at_ms: u64,
    pub fault_id: String,
    pub detection_class: String,
    pub level: String,
    pub stage: Stage,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    source_window_ms: Option<Window>,
    guardian: GuardianSection,
}

#[derive(Deserialize)]
struct GuardianSection {
    #[serde(default)]
    allow_unspecified: bool,
    #[serde(default)]
    transitions: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    at_ms: At,
    class: String,
    level: String,
    state: State,
    #[serde(default)]
    signal: Option<String>,
    #[serde(default)]
    signals: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum At {
    Once(u64),
    Every { from: u64, through: u64, every: u64 },
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum State {
    Active,
    Cleared,
}

pub fn parse(src: &str) -> Result<Oracle> {
    let file: File = serde_yaml::from_str(src).context("invalid oracle")?;
    let mut transitions = Vec::new();
    for e in file.guardian.transitions {
        let times: Vec<u64> = match e.at_ms {
            At::Once(t) => vec![t],
            At::Every { every: 0, .. } => bail!("{} {}: `every` must be > 0", e.class, e.level),
            At::Every {
                from,
                through,
                every,
            } => (from..=through).step_by(every as usize).collect(),
        };
        let signals: Vec<Option<String>> = match (e.signals, e.signal) {
            (Some(list), _) => list.into_iter().map(Some).collect(),
            (None, signal) => vec![signal],
        };
        for &at_ms in &times {
            for signal in &signals {
                transitions.push(Transition {
                    at_ms,
                    class: e.class.clone(),
                    level: e.level.clone(),
                    active: matches!(e.state, State::Active),
                    signal: signal.clone(),
                });
            }
        }
    }
    transitions.sort_by_key(|t| t.at_ms);
    Ok(Oracle {
        allow_unspecified: file.guardian.allow_unspecified,
        transitions,
        source_window_ms: file.source_window_ms,
    })
}

/// Projects the detection transitions onto fault-level changes. Transitions
/// at the same `at_ms` are one Guardian cycle and are applied together, so a
/// signal handing over to another in the same cycle is no change. Returns the
/// expected changes and the number of transitions without a DFM fault.
pub fn project(oracle: &Oracle) -> (Vec<Expected>, usize) {
    let mut active: BTreeMap<&'static str, BTreeSet<Option<String>>> = BTreeMap::new();
    let mut expected = Vec::new();
    let mut not_applicable = 0;
    let mut i = 0;
    while i < oracle.transitions.len() {
        let at_ms = oracle.transitions[i].at_ms;
        let cycle: Vec<&Transition> = oracle.transitions[i..]
            .iter()
            .take_while(|t| t.at_ms == at_ms)
            .collect();
        i += cycle.len();

        let mut before: BTreeMap<&'static str, (bool, &Transition)> = BTreeMap::new();
        for t in cycle {
            let Some(id) = fault_id(&t.class, &t.level) else {
                not_applicable += 1;
                continue;
            };
            let signals = active.entry(id).or_default();
            before.entry(id).or_insert((!signals.is_empty(), t));
            if t.active {
                signals.insert(t.signal.clone());
            } else {
                signals.remove(&t.signal);
            }
        }
        for (id, (was_failed, t)) in before {
            let is_failed = !active[id].is_empty();
            if was_failed != is_failed {
                expected.push(Expected {
                    at_ms,
                    fault_id: id.to_owned(),
                    detection_class: t.class.clone(),
                    level: t.level.clone(),
                    stage: if is_failed {
                        Stage::Failed
                    } else {
                        Stage::Passed
                    },
                });
            }
        }
    }
    (expected, not_applicable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(at_ms: u64, class: &str, active: bool, signal: &str) -> Transition {
        Transition {
            at_ms,
            class: class.into(),
            level: "VIOLATION".into(),
            active,
            signal: Some(signal.into()),
        }
    }

    fn stages(oracle: Vec<Transition>) -> Vec<(u64, Stage)> {
        let (e, _) = project(&Oracle {
            allow_unspecified: false,
            transitions: oracle,
            source_window_ms: None,
        });
        e.iter().map(|e| (e.at_ms, e.stage)).collect()
    }

    #[test]
    fn expands_compact_forms() {
        let o = parse(
            "guardian:
  allow_unspecified: true
  transitions:
    - { at_ms: 3000, class: THERMAL_LIMIT, level: WARNING, state: active, signal: temp_max }
    - class: PHYSICAL_TEMP_RATE
      level: WARNING
      state: cleared
      signals: [temp_min, temp_max]
      at_ms: { from: 700, through: 1900, every: 600 }
",
        )
        .unwrap();
        assert!(o.allow_unspecified);
        // 3 times x 2 signals + 1, sorted by time.
        assert_eq!(o.transitions.len(), 7);
        let times: Vec<u64> = o.transitions.iter().map(|t| t.at_ms).collect();
        assert_eq!(times, [700, 700, 1300, 1300, 1900, 1900, 3000]);
        assert!(!o.transitions[0].active);
    }

    #[test]
    fn fault_stays_failed_until_last_signal_clears() {
        let r = "PHYSICAL_TEMP_RATE";
        assert_eq!(
            stages(vec![
                t(1000, r, true, "temp_min"),
                t(1100, r, true, "temp_max"),
                t(1200, r, false, "temp_min"),
                t(1300, r, false, "temp_max"),
            ]),
            [(1000, Stage::Failed), (1300, Stage::Passed)]
        );
    }

    #[test]
    fn handover_in_one_cycle_is_no_change() {
        let s = "SIGNAL_STUCK";
        assert_eq!(
            stages(vec![
                t(1000, s, true, "temp_avg"),
                t(1200, s, false, "temp_avg"),
                t(1200, s, true, "temp_max"),
                t(1400, s, false, "temp_max"),
            ]),
            [(1000, Stage::Failed), (1400, Stage::Passed)]
        );
    }

    #[test]
    fn unmapped_pairs_are_not_applicable() {
        let mut warning = t(600, "PHYSICAL_TEMP_RATE", true, "temp_min");
        warning.level = "WARNING".into();
        let (e, na) = project(&Oracle {
            allow_unspecified: false,
            transitions: vec![warning],
            source_window_ms: None,
        });
        assert!(e.is_empty());
        assert_eq!(na, 1);
    }

    /// The table must match `DFM_MAPPINGS` in the Guardian source.
    #[test]
    fn mappings_match_guardian() {
        let src = include_str!("../../guardien/source/guardian_faults.rs");
        let block = src
            .split("pub const DFM_MAPPINGS")
            .nth(1)
            .and_then(|s| s.split("];").next())
            .expect("DFM_MAPPINGS in guardian_faults.rs");
        let snake = |camel: &str| {
            let mut out = String::new();
            for (i, c) in camel.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_uppercase());
            }
            out
        };
        let after = |key: &str| -> Vec<String> {
            block
                .split(key)
                .skip(1)
                .map(|s| {
                    s.split(|c: char| !c.is_alphanumeric())
                        .next()
                        .unwrap()
                        .to_owned()
                })
                .collect()
        };
        let classes = after("DetectionClass::");
        let levels = after("DetectionLevel::");
        let ids: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
        let guardian: Vec<(String, String, String)> = classes
            .iter()
            .zip(&levels)
            .zip(&ids)
            .map(|((c, l), id)| (snake(c), snake(l), (*id).to_owned()))
            .collect();
        let ours: Vec<(String, String, String)> = DFM_MAPPINGS
            .iter()
            .map(|(c, l, id)| ((*c).into(), (*l).into(), (*id).into()))
            .collect();
        assert_eq!(ours, guardian);
    }

    #[test]
    fn golden_oracles_parse() {
        for src in [
            include_str!("../../../tests/battery_campaign/scenarios/0_cold_nominal.oracle.yaml"),
            include_str!("../../../tests/battery_campaign/scenarios/1_warm_nominal.oracle.yaml"),
            include_str!("../../../tests/battery_campaign/scenarios/2_hot_nominal.oracle.yaml"),
            include_str!("../../../tests/battery_campaign/scenarios/3_overtemp_fault.oracle.yaml"),
            include_str!("../../../tests/battery_campaign/scenarios/4_hotspot_fault.oracle.yaml"),
        ] {
            parse(src).unwrap();
        }
        let hot = parse(include_str!(
            "../../../tests/battery_campaign/scenarios/2_hot_nominal.oracle.yaml"
        ))
        .unwrap();
        assert_eq!(hot.source_window_ms.map(|w| w.end), Some(19_900));
        let (e, _) = project(&hot);
        assert_eq!(e[0].fault_id, "BatteryOverTempWarning");
        assert_eq!((e[0].at_ms, e[0].stage), (0, Stage::Failed));
    }
}
