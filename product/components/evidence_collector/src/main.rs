//! Evidence collector.
//!
//! Started with a case prefix: reads the transition oracle
//! `<prefix>.oracle.yaml`, the replay `<prefix>.asc` and, if present, the
//! ground truth `<prefix>.ground_truth.yaml` (`<prefix>.json` as fallback),
//! then listens on uProtocol to
//! - `GuardianFaultEvent` (`//guardian/1001/1/8001`): fault-level changes, and
//! - `BatteryTempEvent` (`//battery-vss/9001/1/9001`): the battery stream,
//!
//! and polls the DFM's faults in OpenSOVD (`--sovd-url`, `dfm_sovd_bridge`).
//!
//! Time base (ADR-013): source milliseconds of the replay. A fault event is
//! placed at its `evidence.timestamp_ms` (source time of the causing sample),
//! else at the latest battery `timestamp_ms`. Wall-clock (epoch) timestamps
//! of an old VSS bridge are rebased to the first battery event.
//!
//! Guardian plane: the oracle's detection transitions are projected onto the
//! fault level (see `oracle`). Every expected `Failed`/`Passed` change must
//! arrive within `[at_ms, at_ms + 100]` (Guardian slack); any other change is
//! unexpected and fails the case unless the oracle sets `allow_unspecified`.
//! Detection transitions without a DFM fault are not published and counted
//! as not applicable.
//!
//! DFM/OpenSOVD plane: every Guardian `Failed` change must become visible in
//! OpenSOVD (`test_failed` or a higher `occurrence_counter`) within 500 ms,
//! otherwise the case fails; an unreachable OpenSOVD makes it INCONCLUSIVE.
//! `--no-sovd` skips this plane.
//!
//! Injections (ground truth) only group the result: each owns the expected
//! transitions from its start up to the next injection's start.
//!
//! Collection stops once the battery timeline reaches the end of the replay
//! (last `.asc` frame) plus `--idle-timeout` for late events, or after
//! `--idle-timeout` without any message.
//!
//! Usage:
//!   evidence_collector <prefix> [--oracle FILE] [--report FILE]
//!       [--fault-topic URI] [--battery-topic URI] [--idle-timeout SECS]
//!       [--sovd-url URL | --no-sovd]
//!
//! The JSON report goes to `reports/<case name>.json` (relative to the current
//! directory) unless `--report` names another file.
//! `ZENOH_CONNECT` selects the Zenoh router.
//! Exit code: 0 PASS, 1 FAIL, 2 INCONCLUSIVE, 3 usage/input error.

mod oracle;
mod sovd;
mod uprotocol;

use std::collections::BTreeMap;
use std::fs;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use up_rust::UUri;

#[cfg(test)]
const CONTRACT: &str = include_str!("../../../interfaces/battery_fault_contract.yaml");

/// Slack when comparing the battery timeline with the replay end (a lost
/// last frame).
const REPLAY_END_TOLERANCE_MS: u64 = 200;
/// A Guardian change may arrive up to one evaluation period after the oracle
/// time (`guardian_slack_ms`).
const GUARDIAN_SLACK_MS: u64 = 100;
/// A Guardian failure must be visible in OpenSOVD within this time
/// (`dfm_slack_ms`).
const SOVD_SLACK_MS: u64 = 500;
/// OpenSOVD may appear up to one poll period "early" on the timeline.
const SOVD_EARLY_MS: u64 = 100;
/// Fault the Guardian raises when the battery stream stops.
const STALE_FAULT: &str = "BatteryTempStreamStale";
/// Timestamps above this are epoch milliseconds (wall clock), not source time.
const EPOCH_MS: u64 = 1_000_000_000_000;

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Failed,
    Passed,
}

/// Evidence fields used from a `GuardianFaultEvent`.
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
struct Evidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timestamp_ms: Option<u64>,
}

/// `GuardianFaultEvent` (contract `guardian_fault_event`); unused fields skipped.
#[derive(Deserialize, Serialize, Clone, Debug)]
struct FaultEvent {
    fault_id: String,
    detection_class: String,
    #[serde(default)]
    level: String,
    stage: Stage,
    #[serde(default)]
    baseline: bool,
    #[serde(default)]
    evidence: Evidence,
}

/// `BatteryTempEvent`; only the source timestamp is needed.
#[derive(Deserialize, Clone, Debug)]
struct BatteryEvent {
    timestamp_ms: u64,
    /// When the collector received it (set by the listener).
    #[serde(skip)]
    received: Option<Instant>,
}

#[derive(Debug)]
enum Message {
    Fault(FaultEvent),
    Battery(BatteryEvent),
    /// One OpenSOVD poll (`None` if it failed) and when it was answered.
    Sovd(Option<sovd::Snapshot>, Option<Instant>),
}

/// Decodes a payload as fault event (has `fault_id`) or battery event.
fn decode_message(payload: &[u8]) -> Result<Message> {
    let value: serde_json::Value = serde_json::from_slice(payload)?;
    if value.get("fault_id").is_some() {
        Ok(Message::Fault(serde_json::from_value(value)?))
    } else {
        Ok(Message::Battery(serde_json::from_value(value)?))
    }
}

/// Maps message timestamps onto the source timeline: source timestamps are
/// used as they are, epoch timestamps relative to the first battery event.
#[derive(Default)]
struct Clock {
    epoch_origin: Option<u64>,
}

impl Clock {
    fn battery(&mut self, ts: u64) -> u64 {
        if ts < EPOCH_MS {
            return ts;
        }
        ts - *self.epoch_origin.get_or_insert(ts)
    }

    fn other(&self, ts: u64) -> Option<u64> {
        if ts < EPOCH_MS {
            return Some(ts);
        }
        self.epoch_origin.map(|o| ts.saturating_sub(o))
    }
}

/// One ground-truth record (contract `campaign_ground_truth_event`).
#[derive(Deserialize, Serialize, Clone, Debug)]
struct Injection {
    #[serde(default)]
    run_id: Option<String>,
    injection_id: String,
    injected_class: String,
    /// Injection window in ms on the source timeline (case mutator).
    #[serde(default)]
    source_started_at_ms: Option<u64>,
    #[serde(default)]
    source_finished_at_ms: Option<u64>,
    /// Epoch wall clock from the case mutator (informational), or ms on the
    /// source timeline in hand-written records.
    #[serde(default)]
    started_at: Option<serde_yaml::Value>,
    #[serde(default)]
    finished_at: Option<u64>,
    #[serde(default)]
    duration_ms: Option<u64>,
    /// `[start, finish]` in ms on the source timeline, derived when parsing.
    #[serde(skip)]
    window: (u64, u64),
}

impl Injection {
    /// Window on the source timeline: `source_started_at_ms`/`source_finished_at_ms`,
    /// else a numeric `started_at` with `finished_at` or `duration_ms`.
    fn source_window(&self) -> Result<(u64, u64)> {
        let id = &self.injection_id;
        let start = match (self.source_started_at_ms, &self.started_at) {
            (Some(s), _) => s,
            (None, Some(v)) => v.as_u64().with_context(|| {
                format!("{id}: needs source_started_at_ms (started_at is not ms on the source timeline)")
            })?,
            (None, None) => bail!("{id}: needs source_started_at_ms"),
        };
        let finish = match (self.source_finished_at_ms, self.finished_at, self.duration_ms) {
            (Some(f), _, _) | (None, Some(f), _) => f,
            (None, None, Some(d)) => start + d,
            (None, None, None) => bail!("{id}: needs source_finished_at_ms, finished_at or duration_ms"),
        };
        if finish < start {
            bail!("{id}: finished before it started");
        }
        Ok((start, finish))
    }
}

/// Parses a ground-truth file: one record, a list of records, or empty/`[]`.
/// JSON and YAML are both accepted.
fn parse_ground_truth(src: &str) -> Result<Vec<Injection>> {
    let value: serde_yaml::Value = serde_yaml::from_str(src)?;
    let records = match value {
        serde_yaml::Value::Null => Vec::new(),
        serde_yaml::Value::Sequence(items) => items,
        other => vec![other],
    };
    let mut injections = records
        .into_iter()
        .map(|r| {
            let mut inj: Injection =
                serde_yaml::from_value(r).context("invalid ground-truth record")?;
            inj.window = inj.source_window()?;
            Ok(inj)
        })
        .collect::<Result<Vec<_>>>()?;
    injections.sort_by_key(|i| i.window.0);
    Ok(injections)
}

/// A fault event placed on the source timeline. `t_ms` is `None` if it
/// cannot be placed (no source timestamp, no battery event yet).
#[derive(Serialize, Clone, Debug)]
struct TimedFault {
    t_ms: Option<u64>,
    #[serde(flatten)]
    event: FaultEvent,
}

/// A fault code becoming active in OpenSOVD, placed on the timeline.
#[derive(Serialize, Clone, Debug)]
struct SovdActivation {
    t_ms: Option<u64>,
    code: String,
}

#[derive(Default)]
struct SovdTimeline {
    polls: usize,
    errors: usize,
    activations: Vec<SovdActivation>,
}

struct Timeline {
    faults: Vec<TimedFault>,
    /// Last battery position (`None` if no battery event arrived).
    battery_end: Option<u64>,
    sovd: SovdTimeline,
}

/// Places fault events and OpenSOVD activations on the source timeline.
/// An activation is `test_failed` going true or the occurrence counter rising
/// (one activation per counted occurrence, so short faults between two polls
/// are not lost). A poll is placed at the last battery time plus the time
/// since that battery event arrived, so polls during a stream gap (dropout)
/// still advance on the timeline.
fn timeline(messages: &[Message]) -> Timeline {
    let mut clock = Clock::default();
    let mut now = None;
    let mut last_arrival: Option<Instant> = None;
    let mut faults = Vec::new();
    let mut sovd = SovdTimeline::default();
    let mut previous = sovd::Snapshot::new();
    for m in messages {
        match m {
            Message::Battery(b) => {
                now = Some(clock.battery(b.timestamp_ms));
                last_arrival = b.received;
            }
            Message::Fault(f) => faults.push(TimedFault {
                t_ms: f.evidence.timestamp_ms.and_then(|ts| clock.other(ts)).or(now),
                event: f.clone(),
            }),
            Message::Sovd(None, _) => sovd.errors += 1,
            Message::Sovd(Some(snapshot), polled) => {
                sovd.polls += 1;
                let at = match (now, last_arrival, polled) {
                    (Some(t), Some(a), Some(p)) => Some(t + p.saturating_duration_since(a).as_millis() as u64),
                    _ => now,
                };
                for (code, state) in snapshot {
                    let before = previous.get(code).copied().unwrap_or_default();
                    let counted = state.occurrences.saturating_sub(before.occurrences);
                    let new = if counted > 0 {
                        counted
                    } else {
                        u32::from(state.active && !before.active)
                    };
                    for _ in 0..new {
                        sovd.activations.push(SovdActivation {
                            t_ms: at,
                            code: code.clone(),
                        });
                    }
                }
                previous = snapshot.clone();
            }
        }
    }
    Timeline {
        faults,
        battery_end: now,
        sovd,
    }
}

/// Drops pairs of opposite changes of one fault at the same time: a signal
/// handing over to another within one Guardian cycle can briefly pass the
/// fault (the oracle projection treats the cycle as a whole).
fn collapse_handovers(faults: Vec<TimedFault>) -> Vec<TimedFault> {
    let mut kept: Vec<Option<TimedFault>> = Vec::new();
    let mut last: BTreeMap<String, usize> = BTreeMap::new();
    for f in faults {
        if let Some(&i) = last.get(&f.event.fault_id) {
            if let Some(prev) = &kept[i] {
                if prev.t_ms.is_some() && prev.t_ms == f.t_ms && prev.event.stage != f.event.stage {
                    kept[i] = None;
                    last.remove(&f.event.fault_id);
                    continue;
                }
            }
        }
        last.insert(f.event.fault_id.clone(), kept.len());
        kept.push(Some(f));
    }
    kept.into_iter().flatten().collect()
}

/// Replay length: timestamp of the last `.asc` frame, in ms. Frame lines start
/// with a timestamp; header and `End TriggerBlock` are skipped.
fn asc_end_ms(asc: &str) -> Result<u64> {
    let last_s = asc
        .lines()
        .filter_map(|l| l.split_whitespace().next()?.parse::<f64>().ok())
        .last()
        .context("no frames")?;
    Ok((last_s * 1000.0).round() as u64)
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "UPPERCASE")]
enum Verdict {
    Pass,
    Inconclusive,
    Fail,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Status {
    /// Expected and observed in time.
    Matched,
    /// Expected but not observed.
    Missing,
    /// Expected after the end of the battery stream: no verdict possible.
    NotReached,
    /// Observed but not expected.
    Unexpected,
}

/// One fault-level change: expected by the oracle and/or observed.
#[derive(Serialize, Clone, Debug)]
struct TransitionResult {
    status: Status,
    fault_id: String,
    detection_class: String,
    level: String,
    stage: Stage,
    expected_at_ms: Option<u64>,
    observed_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal: Option<String>,
    /// For `Failed` changes, when the fault became visible in OpenSOVD.
    #[serde(skip_serializing_if = "Option::is_none")]
    sovd_visible_at_ms: Option<u64>,
    /// `Failed` change that never became visible in OpenSOVD in time.
    sovd_missing: bool,
    /// Injection whose slot contains this change.
    #[serde(skip_serializing_if = "Option::is_none")]
    injection_id: Option<String>,
}

impl TransitionResult {
    fn time(&self) -> u64 {
        self.expected_at_ms.or(self.observed_at_ms).unwrap_or(0)
    }

    fn is_mismatch(&self, allow_unspecified: bool) -> bool {
        match self.status {
            Status::Missing => true,
            Status::Unexpected => !allow_unspecified,
            Status::Matched | Status::NotReached => false,
        }
    }
}

#[derive(Serialize)]
struct InjectionResult {
    #[serde(flatten)]
    injection: Injection,
    window_ms: (u64, u64),
    /// Expected changes from this start up to the next injection's start.
    slot_ms: (u64, Option<u64>),
    verdict: Verdict,
    expected: usize,
    matched: usize,
    missing: usize,
    unexpected: usize,
    sovd_missing: usize,
}

#[derive(Serialize)]
struct OracleSummary {
    file: String,
    allow_unspecified: bool,
    /// Fault-level changes the Guardian must publish.
    expected: usize,
    /// Detection transitions without a DFM fault (not published).
    not_applicable: usize,
    matched: usize,
    missing: usize,
    not_reached: usize,
    unexpected: usize,
}

#[derive(Serialize)]
struct SovdReport {
    url: String,
    polls: usize,
    errors: usize,
    /// Fault codes becoming active in OpenSOVD, in order.
    activations: Vec<SovdActivation>,
    /// Guardian failures that never became visible in OpenSOVD in time.
    not_visible: Vec<TimedFault>,
    /// OpenSOVD activations without a matching Guardian failure.
    unexplained: Vec<SovdActivation>,
}

#[derive(Serialize)]
struct Report {
    case: String,
    verdict: Verdict,
    replay_end_ms: u64,
    battery_end_ms: Option<u64>,
    oracle: OracleSummary,
    injections: Vec<InjectionResult>,
    /// Expected and observed fault-level changes, by time.
    transitions: Vec<TransitionResult>,
    /// All non-baseline fault events as received.
    fault_events: Vec<TimedFault>,
    /// OpenSOVD plane (absent with `--no-sovd`).
    sovd: Option<SovdReport>,
    notes: Vec<String>,
}

/// Matches each Guardian `Failed` change to one OpenSOVD activation of its
/// fault code within `[t - SOVD_EARLY_MS, t + SOVD_SLACK_MS]`.
fn match_sovd(faults: &[TimedFault], sovd: &SovdTimeline, url: &str) -> (SovdReport, Vec<Option<u64>>) {
    let mut visible_at = vec![None; faults.len()];
    let mut used = vec![false; sovd.activations.len()];
    let mut not_visible = Vec::new();
    for (i, f) in faults.iter().enumerate() {
        let (Stage::Failed, Some(t)) = (f.event.stage, f.t_ms) else {
            continue;
        };
        let found = sovd.activations.iter().enumerate().position(|(j, a)| {
            !used[j]
                && a.code == f.event.fault_id
                && a.t_ms.is_some_and(|ta| ta + SOVD_EARLY_MS >= t && ta <= t + SOVD_SLACK_MS)
        });
        match found {
            Some(j) => {
                used[j] = true;
                visible_at[i] = sovd.activations[j].t_ms;
            }
            None => not_visible.push(f.clone()),
        }
    }
    let unexplained = sovd
        .activations
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|(a, _)| a.clone())
        .collect();
    let report = SovdReport {
        url: url.to_owned(),
        polls: sovd.polls,
        errors: sovd.errors,
        activations: sovd.activations.clone(),
        not_visible,
        unexplained,
    };
    (report, visible_at)
}

struct Case<'a> {
    name: &'a str,
    oracle_file: &'a str,
    oracle: &'a oracle::Oracle,
    injections: &'a [Injection],
    replay_end_ms: u64,
    sovd_url: Option<&'a str>,
}

fn evaluate(case: &Case, messages: &[Message]) -> Report {
    let Timeline {
        faults: all_faults,
        battery_end,
        sovd,
    } = timeline(messages);
    let all_faults: Vec<TimedFault> = all_faults.into_iter().filter(|f| !f.event.baseline).collect();
    let mut notes = Vec::new();
    let mut verdict = Verdict::Pass;
    // The stream stops with the replay, so the Guardian rightly reports it
    // stale afterwards; the oracle does not cover that.
    let end_of_replay = |f: &TimedFault| {
        f.event.fault_id == STALE_FAULT && f.t_ms.is_some_and(|t| t >= case.replay_end_ms)
    };
    let stale_at_end = all_faults.iter().filter(|f| end_of_replay(f)).count();
    if stale_at_end > 0 {
        notes.push(format!("{STALE_FAULT} at the end of the replay ignored ({stale_at_end} change(s))"));
    }
    let faults = collapse_handovers(all_faults.iter().filter(|f| !end_of_replay(f)).cloned().collect());

    match battery_end {
        None => {
            notes.push("no battery event received; fault events cannot be placed in time".into());
            verdict = Verdict::Inconclusive;
        }
        Some(end) if end + REPLAY_END_TOLERANCE_MS < case.replay_end_ms => {
            notes.push(format!("battery stream ended at {end} ms, replay lasts {} ms", case.replay_end_ms));
            verdict = Verdict::Inconclusive;
        }
        _ => {}
    }

    // DFM/OpenSOVD plane.
    let (sovd_report, visible_at) = match case.sovd_url {
        Some(url) => {
            let (r, v) = match_sovd(&faults, &sovd, url);
            (Some(r), v)
        }
        None => (None, vec![None; faults.len()]),
    };
    let sovd_reachable = sovd_report.as_ref().is_some_and(|r| r.polls > 0);
    if let Some(r) = &sovd_report {
        if r.polls == 0 {
            notes.push(format!("OpenSOVD not reachable at {} ({} failed polls)", r.url, r.errors));
            verdict = verdict.max(Verdict::Inconclusive);
        } else if !r.not_visible.is_empty() {
            notes.push(format!(
                "{} Guardian failure(s) not visible in OpenSOVD within {SOVD_SLACK_MS} ms",
                r.not_visible.len()
            ));
            verdict = Verdict::Fail;
        }
    }

    // Guardian plane: expected changes against observed ones.
    let (expected, not_applicable) = oracle::project(case.oracle);
    let mut used = vec![false; faults.len()];
    let mut transitions = Vec::new();
    for e in &expected {
        let found = faults.iter().enumerate().position(|(i, f)| {
            !used[i]
                && f.event.fault_id == e.fault_id
                && f.event.stage == e.stage
                && f.t_ms.is_some_and(|t| e.at_ms <= t && t <= e.at_ms + GUARDIAN_SLACK_MS)
        });
        if let Some(i) = found {
            used[i] = true;
        }
        let status = match (found, battery_end) {
            (Some(_), _) => Status::Matched,
            (None, Some(end)) if e.at_ms + GUARDIAN_SLACK_MS <= end => Status::Missing,
            (None, _) => Status::NotReached,
        };
        transitions.push(TransitionResult {
            status,
            fault_id: e.fault_id.clone(),
            detection_class: e.detection_class.clone(),
            level: e.level.clone(),
            stage: e.stage,
            expected_at_ms: Some(e.at_ms),
            observed_at_ms: found.and_then(|i| faults[i].t_ms),
            signal: found.and_then(|i| faults[i].event.evidence.signal.clone()),
            sovd_visible_at_ms: found.and_then(|i| visible_at[i]),
            sovd_missing: found.is_some_and(|i| {
                sovd_reachable && e.stage == Stage::Failed && visible_at[i].is_none()
            }),
            injection_id: None,
        });
    }
    let unplaced = faults.iter().filter(|f| f.t_ms.is_none()).count();
    if unplaced > 0 {
        notes.push(format!("{unplaced} fault event(s) without time (before the battery stream)"));
        verdict = verdict.max(Verdict::Inconclusive);
    }
    for (i, f) in faults.iter().enumerate().filter(|(i, f)| !used[*i] && f.t_ms.is_some()) {
        transitions.push(TransitionResult {
            status: Status::Unexpected,
            fault_id: f.event.fault_id.clone(),
            detection_class: f.event.detection_class.clone(),
            level: f.event.level.clone(),
            stage: f.event.stage,
            expected_at_ms: None,
            observed_at_ms: f.t_ms,
            signal: f.event.evidence.signal.clone(),
            sovd_visible_at_ms: visible_at[i],
            sovd_missing: sovd_reachable && f.event.stage == Stage::Failed && visible_at[i].is_none(),
            injection_id: None,
        });
    }
    transitions.sort_by_key(|t| t.time());

    // Injections own the changes from their start up to the next start.
    let slot = |i: usize| {
        let start = case.injections[i].window.0;
        (start, case.injections.get(i + 1).map(|n| n.window.0))
    };
    for t in &mut transitions {
        let at = t.time();
        t.injection_id = (0..case.injections.len())
            .rev()
            .find(|&i| slot(i).0 <= at)
            .map(|i| case.injections[i].injection_id.clone());
    }

    let allow = case.oracle.allow_unspecified;
    let count = |s: Status| transitions.iter().filter(|t| t.status == s).count();
    let summary = OracleSummary {
        file: case.oracle_file.to_owned(),
        allow_unspecified: allow,
        expected: expected.len(),
        not_applicable,
        matched: count(Status::Matched),
        missing: count(Status::Missing),
        not_reached: count(Status::NotReached),
        unexpected: count(Status::Unexpected),
    };
    if summary.missing > 0 {
        notes.push(format!("{} expected Guardian change(s) missing", summary.missing));
        verdict = Verdict::Fail;
    }
    if summary.unexpected > 0 {
        if allow {
            notes.push(format!("{} unexpected Guardian change(s), allowed by the oracle", summary.unexpected));
        } else {
            notes.push(format!("{} unexpected Guardian change(s)", summary.unexpected));
            verdict = Verdict::Fail;
        }
    }
    if summary.not_reached > 0 {
        notes.push(format!("{} expected change(s) after the end of the battery stream", summary.not_reached));
        verdict = verdict.max(Verdict::Inconclusive);
    }

    let injections = case
        .injections
        .iter()
        .enumerate()
        .map(|(i, inj)| {
            let own: Vec<&TransitionResult> = transitions
                .iter()
                .filter(|t| t.injection_id.as_deref() == Some(inj.injection_id.as_str()))
                .collect();
            let n = |s: Status| own.iter().filter(|t| t.status == s).count();
            let sovd_missing = own.iter().filter(|t| t.sovd_missing).count();
            let result = if own.iter().any(|t| t.is_mismatch(allow)) || sovd_missing > 0 {
                Verdict::Fail
            } else if n(Status::NotReached) > 0 {
                Verdict::Inconclusive
            } else {
                Verdict::Pass
            };
            InjectionResult {
                injection: inj.clone(),
                window_ms: inj.window,
                slot_ms: slot(i),
                verdict: result,
                expected: own.iter().filter(|t| t.expected_at_ms.is_some()).count(),
                matched: n(Status::Matched),
                missing: n(Status::Missing),
                unexpected: n(Status::Unexpected),
                sovd_missing,
            }
        })
        .collect();

    Report {
        case: case.name.to_owned(),
        verdict,
        replay_end_ms: case.replay_end_ms,
        battery_end_ms: battery_end,
        oracle: summary,
        injections,
        transitions,
        fault_events: all_faults,
        sovd: sovd_report,
        notes,
    }
}

struct Args {
    prefix: String,
    oracle: Option<String>,
    report: Option<String>,
    fault_topic: String,
    battery_topic: String,
    idle_timeout: Duration,
    sovd_url: Option<String>,
}

const USAGE: &str = "usage: evidence_collector <prefix> [--oracle FILE] [--report FILE] \
[--fault-topic URI] [--battery-topic URI] [--idle-timeout SECS] [--sovd-url URL | --no-sovd]";

fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let mut prefix = None;
    let (mut oracle, mut report) = (None, None);
    let mut fault_topic = uprotocol::DEFAULT_FAULT_TOPIC.to_owned();
    let mut battery_topic = uprotocol::DEFAULT_BATTERY_TOPIC.to_owned();
    let mut idle_timeout = Duration::from_secs(3);
    let mut sovd_url = Some(sovd::DEFAULT_URL.to_owned());
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--oracle" => oracle = Some(value()?),
            "--report" => report = Some(value()?),
            "--fault-topic" => fault_topic = value()?,
            "--battery-topic" => battery_topic = value()?,
            "--sovd-url" => sovd_url = Some(value()?),
            "--no-sovd" => sovd_url = None,
            "--idle-timeout" => {
                idle_timeout = Duration::from_secs_f64(value()?.parse().context("--idle-timeout")?)
            }
            "-h" | "--help" => bail!(USAGE),
            _ if prefix.is_none() => prefix = Some(arg),
            _ => bail!("unexpected argument: {arg}\n{USAGE}"),
        }
    }
    Ok(Args {
        prefix: prefix.context(USAGE)?,
        oracle,
        report,
        fault_topic,
        battery_topic,
        idle_timeout,
        sovd_url,
    })
}

fn print_report(report: &Report) {
    let end = report.battery_end_ms.map_or("-".into(), |e| format!("{e} ms"));
    println!("{}: {:?} (battery timeline up to {end}, replay {} ms)", report.case, report.verdict, report.replay_end_ms);
    let o = &report.oracle;
    println!(
        "  oracle: {} expected change(s): {} matched, {} missing, {} not reached; {} unexpected{}; \
         {} detection transition(s) without DFM fault",
        o.expected,
        o.matched,
        o.missing,
        o.not_reached,
        o.unexpected,
        if o.allow_unspecified { " (allowed)" } else { "" },
        o.not_applicable
    );
    for r in &report.injections {
        let (s, f) = r.window_ms;
        println!(
            "  {} ({}) [{s}, {f}] ms -> {:?}: {}/{} matched, {} missing, {} unexpected{}",
            r.injection.injection_id,
            r.injection.injected_class,
            r.verdict,
            r.matched,
            r.expected,
            r.missing,
            r.unexpected,
            if r.sovd_missing > 0 { format!(", {} not in OpenSOVD", r.sovd_missing) } else { String::new() }
        );
    }
    println!("  changes (expected / observed):");
    for t in &report.transitions {
        let ms = |v: Option<u64>| v.map_or("-".into(), |v| v.to_string());
        let status = match t.status {
            Status::Matched => "ok",
            Status::Missing => "MISSING",
            Status::NotReached => "not reached",
            Status::Unexpected => "UNEXPECTED",
        };
        let sovd = match (&report.sovd, t.stage, t.sovd_visible_at_ms) {
            (Some(_), Stage::Failed, Some(at)) => format!("OpenSOVD {at} ms"),
            (Some(_), Stage::Failed, None) if t.status != Status::Missing => "<- not in OpenSOVD".into(),
            _ => String::new(),
        };
        println!(
            "    {:>6} / {:>6} ms  {:<26} {:<6} {:<11} {:<9} {}",
            ms(t.expected_at_ms),
            ms(t.observed_at_ms),
            t.fault_id,
            format!("{:?}", t.stage),
            status,
            t.signal.as_deref().unwrap_or("-"),
            sovd
        );
    }
    if let Some(r) = &report.sovd {
        println!(
            "  OpenSOVD: {} polls ({} failed), {} activation(s), {} failure(s) not visible, {} unexplained",
            r.polls,
            r.errors,
            r.activations.len(),
            r.not_visible.len(),
            r.unexplained.len()
        );
    }
    for note in &report.notes {
        println!("  note: {note}");
    }
}

async fn run() -> Result<Verdict> {
    let args = parse_args()?;

    let oracle_file = args.oracle.clone().unwrap_or_else(|| format!("{}.oracle.yaml", args.prefix));
    let oracle = oracle::parse(&fs::read_to_string(&oracle_file).with_context(|| format!("reading {oracle_file}"))?)
        .with_context(|| format!("parsing {oracle_file}"))?;

    let candidates = [
        format!("{}.ground_truth.yaml", args.prefix),
        format!("{}.json", args.prefix),
    ];
    let injections = match candidates.iter().find(|p| std::path::Path::new(p).exists()) {
        Some(file) => parse_ground_truth(&fs::read_to_string(file).with_context(|| format!("reading {file}"))?)
            .with_context(|| format!("parsing {file}"))?,
        None => Vec::new(),
    };

    let replay = format!("{}.asc", args.prefix);
    let replay_end_ms = asc_end_ms(&fs::read_to_string(&replay).with_context(|| format!("reading {replay}"))?)
        .with_context(|| format!("parsing {replay}"))?;
    eprintln!(
        "replay lasts {replay_end_ms} ms, {} injection(s), {} oracle transition(s)",
        injections.len(),
        oracle.transitions.len()
    );

    let topics = [&args.fault_topic, &args.battery_topic]
        .into_iter()
        .map(|t| UUri::from_str(t).with_context(|| format!("invalid topic {t}")))
        .collect::<Result<Vec<_>>>()?;
    let stop = uprotocol::StopCondition {
        end_ms: replay_end_ms,
        idle_timeout: args.idle_timeout,
    };
    let messages = uprotocol::collect(&topics, args.sovd_url.as_deref(), &stop).await?;

    let report = evaluate(
        &Case {
            name: &args.prefix,
            oracle_file: &oracle_file,
            oracle: &oracle,
            injections: &injections,
            replay_end_ms,
            sovd_url: args.sovd_url.as_deref(),
        },
        &messages,
    );
    print_report(&report);

    let path = match &args.report {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let case = std::path::Path::new(&args.prefix)
                .file_name()
                .map_or_else(|| "report".into(), |n| n.to_string_lossy().into_owned());
            std::path::Path::new("reports").join(format!("{case}.json"))
        }
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(&path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("  report: {}", path.display());
    Ok(report.verdict)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(Verdict::Pass) => ExitCode::from(0),
        Ok(Verdict::Fail) => ExitCode::from(1),
        Ok(Verdict::Inconclusive) => ExitCode::from(2),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(3)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const END: u64 = 19_900;

    fn battery(t: u64) -> Message {
        Message::Battery(BatteryEvent {
            timestamp_ms: t,
            received: None,
        })
    }

    /// Fault event as the Guardian publishes it; `ts` is `evidence.timestamp_ms`.
    fn fault(class: &str, stage: Stage, ts: Option<u64>) -> Message {
        Message::Fault(FaultEvent {
            fault_id: oracle::fault_id(class, "VIOLATION").unwrap().into(),
            detection_class: class.into(),
            level: "VIOLATION".into(),
            stage,
            baseline: false,
            evidence: Evidence {
                signal: Some("temp_min".into()),
                timestamp_ms: ts,
            },
        })
    }

    fn copy(m: &Message) -> Message {
        match m {
            Message::Fault(f) => Message::Fault(f.clone()),
            Message::Battery(b) => Message::Battery(b.clone()),
            Message::Sovd(s, at) => Message::Sovd(s.clone(), *at),
        }
    }

    /// Battery events every 100 ms from `first` (source time + `offset`) up
    /// to `END`, with `extra` messages right after the battery event at the
    /// given source time.
    fn stream_from(first: u64, offset: u64, extra: &[(u64, Message)]) -> Vec<Message> {
        let mut out = Vec::new();
        for t in (first..=END).step_by(100) {
            out.push(battery(offset + t));
            out.extend(extra.iter().filter(|(at, _)| *at == t).map(|(_, m)| copy(m)));
        }
        out
    }

    fn stream(extra: &[(u64, Message)]) -> Vec<Message> {
        stream_from(0, 0, extra)
    }

    /// Absolute limit on temp_min active 1000..1500 ms, injection at 1000 ms.
    const ORACLE: &str = "
guardian:
  allow_unspecified: false
  transitions:
    - { at_ms: 1000, class: PHYSICAL_TEMP_ABSOLUTE_LIMIT, level: VIOLATION, state: active, signal: temp_min }
    - { at_ms: 1500, class: PHYSICAL_TEMP_ABSOLUTE_LIMIT, level: VIOLATION, state: cleared, signal: temp_min }
    - { at_ms: 1000, class: PHYSICAL_TEMP_RATE, level: WARNING, state: active, signal: temp_min }
";
    const GROUND_TRUTH: &str = "
- injection_id: oor-1
  injected_class: signal.out_of_range
  source_started_at_ms: 1000
  source_finished_at_ms: 1500
";
    const LIMIT: &str = "PHYSICAL_TEMP_ABSOLUTE_LIMIT";

    fn limit_ok() -> Vec<(u64, Message)> {
        vec![
            (1000, fault(LIMIT, Stage::Failed, Some(1000))),
            (1500, fault(LIMIT, Stage::Passed, Some(1500))),
        ]
    }

    fn check_with(oracle_src: &str, sovd_url: Option<&str>, messages: &[Message]) -> Report {
        let oracle = oracle::parse(oracle_src).unwrap();
        let injections = parse_ground_truth(GROUND_TRUTH).unwrap();
        evaluate(
            &Case {
                name: "test",
                oracle_file: "test.oracle.yaml",
                oracle: &oracle,
                injections: &injections,
                replay_end_ms: END,
                sovd_url,
            },
            messages,
        )
    }

    fn check(messages: &[Message]) -> Report {
        check_with(ORACLE, None, messages)
    }

    fn statuses(r: &Report) -> Vec<(Option<u64>, Status)> {
        r.transitions.iter().map(|t| (t.expected_at_ms, t.status)).collect()
    }

    #[test]
    fn expected_changes_in_time_pass() {
        let r = check(&stream(&limit_ok()));
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.notes);
        assert_eq!(r.oracle.expected, 2);
        assert_eq!(r.oracle.not_applicable, 1); // the rate WARNING
        assert_eq!(r.injections[0].verdict, Verdict::Pass);
        assert_eq!(r.injections[0].matched, 2);
    }

    #[test]
    fn guardian_slack_is_right_hand_only() {
        for (at, ok) in [(1000, true), (1100, true), (1101, false), (999, false)] {
            let m = stream(&[
                (1000, fault(LIMIT, Stage::Failed, Some(at))),
                (1500, fault(LIMIT, Stage::Passed, Some(1500))),
            ]);
            let r = check(&m);
            assert_eq!(r.verdict == Verdict::Pass, ok, "Failed at {at} ms");
        }
    }

    #[test]
    fn missing_change_fails() {
        let r = check(&stream(&limit_ok()[..1]));
        assert_eq!(r.verdict, Verdict::Fail);
        assert_eq!(statuses(&r)[1], (Some(1500), Status::Missing));
        assert_eq!(r.injections[0].verdict, Verdict::Fail);
    }

    #[test]
    fn unexpected_change_fails_unless_allowed() {
        let mut extra = limit_ok();
        extra.push((3000, fault("SIGNAL_STUCK", Stage::Failed, Some(3000))));
        let r = check(&stream(&extra));
        assert_eq!(r.verdict, Verdict::Fail);
        assert_eq!(r.oracle.unexpected, 1);
        assert_eq!(r.injections[0].unexpected, 1); // inside its slot (open end)
        let allowed = ORACLE.replace("allow_unspecified: false", "allow_unspecified: true");
        assert_eq!(check_with(&allowed, None, &stream(&extra)).verdict, Verdict::Pass);
    }

    #[test]
    fn empty_oracle_is_a_baseline() {
        let empty = "guardian:\n  allow_unspecified: false\n  transitions: []\n";
        assert_eq!(check_with(empty, None, &stream(&[])).verdict, Verdict::Pass);
        let r = check_with(empty, None, &stream(&[(500, fault(LIMIT, Stage::Failed, Some(500)))]));
        assert_eq!(r.verdict, Verdict::Fail);
    }

    #[test]
    fn stale_after_replay_end_is_ignored() {
        let mut extra = limit_ok();
        extra.push((END, fault("STREAM_STALE", Stage::Failed, Some(END))));
        let r = check(&stream(&extra));
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.notes);
        let mut early = limit_ok();
        early.push((3000, fault("STREAM_STALE", Stage::Failed, Some(3000))));
        assert_eq!(check(&stream(&early)).verdict, Verdict::Fail);
    }

    #[test]
    fn handover_glitch_is_ignored() {
        let mut extra = limit_ok();
        extra.push((1200, fault(LIMIT, Stage::Passed, Some(1200))));
        extra.push((1200, fault(LIMIT, Stage::Failed, Some(1200))));
        assert_eq!(check(&stream(&extra)).verdict, Verdict::Pass);
    }

    #[test]
    fn battery_time_used_without_evidence_timestamp() {
        let m = stream(&[
            (1000, fault(LIMIT, Stage::Failed, None)),
            (1500, fault(LIMIT, Stage::Passed, None)),
        ]);
        assert_eq!(check(&m).verdict, Verdict::Pass);
    }

    #[test]
    fn source_time_is_not_rebased_when_first_frames_are_lost() {
        let m = stream_from(300, 0, &[
            (1000, fault(LIMIT, Stage::Failed, None)),
            (1500, fault(LIMIT, Stage::Passed, None)),
        ]);
        let r = check(&m);
        assert_eq!(r.verdict, Verdict::Pass);
        assert_eq!(r.transitions[0].observed_at_ms, Some(1000));
    }

    #[test]
    fn wall_clock_timestamps_are_rebased() {
        let epoch = 1_791_300_000_000;
        let m = stream_from(0, epoch, &[
            (1000, fault(LIMIT, Stage::Failed, Some(epoch + 1000))),
            (1500, fault(LIMIT, Stage::Passed, None)),
        ]);
        let r = check(&m);
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.notes);
        assert_eq!(r.battery_end_ms, Some(END));
    }

    #[test]
    fn short_stream_is_inconclusive() {
        let m: Vec<_> = stream(&[]).into_iter().take(10).collect(); // up to 900 ms
        let r = check(&m);
        assert_eq!(r.verdict, Verdict::Inconclusive);
        assert_eq!(r.oracle.not_reached, 2);
        assert_eq!(r.injections[0].verdict, Verdict::Inconclusive);
    }

    #[test]
    fn no_battery_events_is_inconclusive() {
        let r = check(&[fault(LIMIT, Stage::Failed, None)]);
        assert_eq!(r.verdict, Verdict::Inconclusive);
    }

    #[test]
    fn baseline_events_are_ignored() {
        let mut m = stream(&limit_ok());
        m.insert(0, Message::Fault(FaultEvent {
            fault_id: "BatteryTempStreamStale".into(),
            detection_class: "STREAM_STALE".into(),
            level: "VIOLATION".into(),
            stage: Stage::Passed,
            baseline: true,
            evidence: Evidence::default(),
        }));
        assert_eq!(check(&m).verdict, Verdict::Pass);
    }

    #[test]
    fn injections_own_their_slot() {
        let gt = "
- { injection_id: a, injected_class: signal.out_of_range, source_started_at_ms: 1000, source_finished_at_ms: 1500 }
- { injection_id: b, injected_class: signal.out_of_range, source_started_at_ms: 4500, source_finished_at_ms: 5000 }
";
        let oracle = oracle::parse(ORACLE).unwrap();
        let injections = parse_ground_truth(gt).unwrap();
        let mut extra = limit_ok();
        extra.push((4600, fault("SIGNAL_STUCK", Stage::Failed, Some(4600))));
        let r = evaluate(
            &Case {
                name: "t",
                oracle_file: "o",
                oracle: &oracle,
                injections: &injections,
                replay_end_ms: END,
                sovd_url: None,
            },
            &stream(&extra),
        );
        assert_eq!(r.injections[0].verdict, Verdict::Pass);
        assert_eq!(r.injections[1].verdict, Verdict::Fail);
        assert_eq!(r.injections[0].slot_ms, (1000, Some(4500)));
    }

    fn sovd(code: &str, active: bool, occurrences: u32) -> Message {
        Message::Sovd(
            Some(sovd::Snapshot::from([(
                code.to_owned(),
                sovd::FaultState { active, occurrences },
            )])),
            None,
        )
    }

    fn with_sovd(sovd_extra: Vec<(u64, Message)>) -> Vec<Message> {
        let mut extra = limit_ok();
        extra.extend(sovd_extra);
        let mut m = stream(&extra);
        m.insert(0, sovd("BatteryTempAbsoluteLimit", false, 0));
        m
    }

    #[test]
    fn sovd_visible_within_slack_passes() {
        let r = check_with(ORACLE, Some("http://sovd"), &with_sovd(vec![(1300, sovd("BatteryTempAbsoluteLimit", true, 1))]));
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.notes);
        assert_eq!(r.transitions[0].sovd_visible_at_ms, Some(1300));
    }

    #[test]
    fn sovd_too_late_or_missing_fails() {
        for extra in [vec![(1600, sovd("BatteryTempAbsoluteLimit", true, 1))], vec![]] {
            let r = check_with(ORACLE, Some("http://sovd"), &with_sovd(extra));
            assert_eq!(r.verdict, Verdict::Fail);
            assert_eq!(r.injections[0].sovd_missing, 1);
            assert!(r.transitions[0].sovd_missing);
        }
    }

    #[test]
    fn sovd_short_fault_counted_by_occurrence_counter() {
        let r = check_with(ORACLE, Some("http://sovd"), &with_sovd(vec![(1100, sovd("BatteryTempAbsoluteLimit", false, 1))]));
        assert_eq!(r.verdict, Verdict::Pass);
    }

    /// Stale during a dropout: no battery events, but OpenSOVD polls go on.
    #[test]
    fn sovd_polls_during_a_gap_advance_on_the_timeline() {
        let stale_oracle = "
guardian:
  allow_unspecified: false
  transitions:
    - { at_ms: 8500, class: STREAM_STALE, level: VIOLATION, state: active }
";
        let base = Instant::now();
        let at = |ms: u64| Some(base + Duration::from_millis(ms));
        let mut m = vec![sovd("BatteryTempStreamStale", false, 0)];
        for t in (0..=7900).step_by(100) {
            m.push(Message::Battery(BatteryEvent { timestamp_ms: t, received: at(t) }));
        }
        m.push(fault("STREAM_STALE", Stage::Failed, Some(8500)));
        let Message::Sovd(snapshot, _) = sovd("BatteryTempStreamStale", true, 1) else { unreachable!() };
        m.push(Message::Sovd(snapshot, at(8550)));
        for t in (8600..=END).step_by(100) {
            m.push(Message::Battery(BatteryEvent { timestamp_ms: t, received: at(t) }));
        }
        let r = check_with(stale_oracle, Some("http://sovd"), &m);
        assert_eq!(r.transitions[0].sovd_visible_at_ms, Some(8550));
        assert!(r.sovd.unwrap().not_visible.is_empty());
    }

    #[test]
    fn sovd_unreachable_is_inconclusive() {
        let mut m = stream(&limit_ok());
        m.insert(0, Message::Sovd(None, None));
        let r = check_with(ORACLE, Some("http://sovd"), &m);
        assert_eq!(r.verdict, Verdict::Inconclusive);
        assert_eq!(r.sovd.unwrap().errors, 1);
    }

    #[test]
    fn ground_truth_formats() {
        assert!(parse_ground_truth("[]").unwrap().is_empty());
        assert!(parse_ground_truth("").unwrap().is_empty());
        let json = r#"{"injection_id": "s", "injected_class": "signal.stuck", "started_at": 5000, "duration_ms": 2000}"#;
        assert_eq!(parse_ground_truth(json).unwrap()[0].window, (5000, 7000));
        let epoch_only = r#"{"injection_id": "s", "injected_class": "signal.spike",
                             "started_at": "2026-10-07T12:00:00Z", "duration_ms": 100}"#;
        assert!(parse_ground_truth(epoch_only).is_err());
    }

    #[test]
    fn decodes_guardian_and_battery_payloads() {
        let fault = br#"{"fault_id":"BatteryTempRate","detection_class":"PHYSICAL_TEMP_RATE",
            "level":"VIOLATION","stage":"Failed","baseline":false,"sovd_path":"battery_guardian",
            "source":{"entity":"BatteryThermalGuardian"},"evidence":{"signal":"temp_max","timestamp_ms":1200}}"#;
        let Message::Fault(f) = decode_message(fault).unwrap() else { panic!("not a fault") };
        assert_eq!((f.stage, f.evidence.timestamp_ms), (Stage::Failed, Some(1200)));
        let baseline = br#"{"fault_id":"BatteryTempRate","detection_class":"PHYSICAL_TEMP_RATE",
            "level":"VIOLATION","stage":"Passed","baseline":true,"evidence":{}}"#;
        assert!(matches!(decode_message(baseline).unwrap(), Message::Fault(f) if f.baseline));
        let battery = br#"{"temp_max":32.5,"temp_avg":30.5,"temp_min":25.5,"soc":67.0,"timestamp_ms":1200}"#;
        assert!(matches!(decode_message(battery).unwrap(), Message::Battery(b) if b.timestamp_ms == 1200));
    }

    #[test]
    fn golden_asc_lasts_19900_ms() {
        let asc = include_str!("../../../tests/battery_campaign/scenarios/0_cold_nominal.asc");
        assert_eq!(asc_end_ms(asc).unwrap(), 19_900);
    }
}
