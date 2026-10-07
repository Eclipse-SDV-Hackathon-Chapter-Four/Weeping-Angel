//! Evidence collector.
//!
//! Started with a case prefix: reads the transition oracle
//! `<prefix>.oracle.yaml`, the replay `<prefix>.asc` and, if present, the
//! ground truth `<prefix>.ground_truth.yaml` (`<prefix>.json` as fallback),
//! then listens on uProtocol to
//! - `GuardianFaultEvent` (`//guardian/1001/1/8001`): fault-level changes,
//! - `GuardianEvidenceEvent` (`//guardian-vss/9000/1/9003`): raw Guardian
//!   detection transitions (mapped or not), counted in the report notes and
//!   never part of the verdict (ADR-007 raw decision stream), and
//! - `BatteryTempEvent` (`//battery-vss/9001/1/9001`): the battery stream,
//!
//! and polls the DFM's faults in OpenSOVD (`--sovd-url`, `dfm_sovd_bridge`).
//!
//! Time base: the replay timeline (ms of the `.asc`), the timeline of the
//! oracle. For each frame the `.asc` gives its replay time and its source
//! TimeStamp (ADR-011/013); both differ only under `transport.delay`. A
//! battery event is placed at its frame's replay time, a fault event at the
//! replay time of its causing frame (`evidence.timestamp_ms`) or, for a
//! projected time without a frame (STREAM_STALE), at the last frame's replay
//! time plus the projected age. Wall-clock (epoch) timestamps of an old VSS
//! bridge are rebased to the first battery event.
//!
//! Guardian plane: the oracle's detection transitions are projected onto the
//! fault level (see `oracle`). Every expected `Failed`/`Passed` change must
//! arrive within `[at_ms, at_ms + 100]` (Guardian slack); any other change is
//! unexpected and fails the case unless the oracle sets `allow_unspecified`.
//! Guardian events strictly after the end of the covered stream (oracle
//! `source_window_ms.end`, else the last ground-truth finish) are
//! post-horizon: reported, but not verdict-relevant — absent both sources the
//! old fail-closed behaviour applies.
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

#[cfg(feature = "observer")]
pub(crate) mod observer;

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
pub(crate) enum Stage {
    Failed,
    Passed,
}

/// Guardian evidence carried by a `GuardianFaultEvent` (contract
/// `guardian_fault_event.evidence`); all fields are optional.
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub(crate) struct FaultEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residual: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utilization: Option<f64>,
    /// Source/generation interval Δτ of a temporal check (ADR-015).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    /// Source timestamp of the causing sample (ADR-013).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<u64>,
}

/// `GuardianFaultEvent` (contract `guardian_fault_event`); unused fields skipped.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub(crate) struct FaultEvent {
    pub fault_id: String,
    pub detection_class: String,
    #[serde(default)]
    pub level: String,
    pub stage: Stage,
    #[serde(default)]
    pub baseline: bool,
    #[serde(default)]
    pub evidence: FaultEvidence,
}

/// `BatteryTempEvent`; the source timestamp is the common time base (ADR-013),
/// the signal values feed the live observer (ADR-016).
#[derive(Deserialize, Clone, Debug, Default)]
#[cfg_attr(not(feature = "observer"), allow(dead_code))]
pub(crate) struct BatteryEvent {
    pub timestamp_ms: u64,
    #[serde(default)]
    pub temp_min: Option<f64>,
    #[serde(default)]
    pub temp_avg: Option<f64>,
    #[serde(default)]
    pub temp_max: Option<f64>,
    #[serde(default)]
    pub soc: Option<f64>,
    /// When the collector received it (set by the listener).
    #[serde(skip)]
    received: Option<Instant>,
}

/// `GuardianEvidenceEvent` (contract `guardian_evidence_event`): one raw
/// Guardian detection transition, mapped or not (ADR-007). The raw stream is
/// statistics-only evidence; it never feeds the ADR-012 verdict logic.
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub(crate) struct EvidenceEvent {
    pub run_id: String,
    pub detection_class: String,
    #[serde(default)]
    pub level: String,
    /// `active` | `cleared` (detection state).
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub signal: Option<String>,
    /// Source timestamp of the causing sample (ADR-017); absent for
    /// transitions without one (baseline/all-clear only — STREAM_STALE is
    /// anchored to its projected staleness deadline on the source timeline).
    #[serde(default)]
    pub timestamp_ms: Option<u64>,
}

#[derive(Debug)]
pub(crate) enum Message {
    Fault(FaultEvent),
    /// Raw Guardian detection transition (ADR-007 raw evidence stream).
    Evidence(EvidenceEvent),
    Battery(BatteryEvent),
    /// One OpenSOVD poll (`None` if it failed) and when it was answered.
    Sovd(Option<sovd::Snapshot>, Option<Instant>),
}

/// Decodes a payload as fault event (has `fault_id`), raw evidence event
/// (has `run_id` + `detection_class`), or battery event.
fn decode_message(payload: &[u8]) -> Result<Message> {
    let value: serde_json::Value = serde_json::from_slice(payload)?;
    if value.get("fault_id").is_some() {
        Ok(Message::Fault(serde_json::from_value(value)?))
    } else if value.get("detection_class").is_some() && value.get("run_id").is_some() {
        Ok(Message::Evidence(serde_json::from_value(value)?))
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
    /// Executed mutations as written by the case mutator; passed through as-is
    /// for the observer's incident markers.
    #[serde(default)]
    mutations: Vec<serde_yaml::Value>,
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
        let finish = match (
            self.source_finished_at_ms,
            self.finished_at,
            self.duration_ms,
        ) {
            (Some(f), _, _) | (None, Some(f), _) => f,
            (None, None, Some(d)) => start + d,
            (None, None, None) => {
                bail!("{id}: needs source_finished_at_ms, finished_at or duration_ms")
            }
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
    /// Raw evidence events, counted but not judged (ADR-007 raw stream).
    evidence: Vec<EvidenceEvent>,
}

/// Places battery and fault events and OpenSOVD activations on the replay
/// timeline (see the module doc). An activation is `test_failed` going true
/// or the occurrence counter rising (one activation per counted occurrence,
/// so short faults between two polls are not lost). A poll is placed at the
/// last battery time plus the time since that battery event arrived, so polls
/// during a stream gap (dropout, delay) still advance on the timeline.
fn timeline(messages: &[Message], replay: &Replay) -> Timeline {
    let mut clock = Clock::default();
    let mut now = None;
    let mut last_source: Option<u64> = None;
    let mut last_arrival: Option<Instant> = None;
    let mut faults = Vec::new();
    let mut evidence = Vec::new();
    let mut sovd = SovdTimeline::default();
    let mut previous = sovd::Snapshot::new();
    for m in messages {
        match m {
            Message::Battery(b) => {
                let source = clock.battery(b.timestamp_ms);
                now = Some(replay.arrival(source).unwrap_or(source));
                last_source = Some(source);
                last_arrival = b.received;
            }
            Message::Evidence(e) => evidence.push(e.clone()),
            Message::Fault(f) => {
                let t_ms = match (
                    f.evidence.timestamp_ms.and_then(|ts| clock.other(ts)),
                    last_source,
                    now,
                ) {
                    // Later than any frame received: a projected time (stale).
                    (Some(e), Some(ls), Some(n)) if e > ls => Some(n + (e - ls)),
                    (Some(e), _, _) => Some(replay.arrival(e).unwrap_or_else(|| replay.nominal(e))),
                    (None, _, n) => n,
                };
                faults.push(TimedFault { t_ms, event: f.clone() });
            }
            Message::Sovd(None, _) => sovd.errors += 1,
            Message::Sovd(Some(snapshot), polled) => {
                sovd.polls += 1;
                let at = match (now, last_arrival, polled) {
                    (Some(t), Some(a), Some(p)) => {
                        Some(t + p.saturating_duration_since(a).as_millis() as u64)
                    }
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
        evidence,
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

/// The replay's frames: when (replay ms) each source TimeStamp is sent.
/// Under `transport.delay` the two diverge (frames held back); otherwise they
/// are equal. Without timestamped frames the mapping is the identity.
#[derive(Default, Debug)]
struct Replay {
    /// source TimeStamp -> replay ms of that frame.
    by_source: BTreeMap<u64, u64>,
}

/// CAN id of the battery frame (DBC `BO_ 256`); TimeStamp = bytes 0..4, LE.
const BATTERY_CAN_ID: &str = "100";

impl Replay {
    fn parse(asc: &str) -> Self {
        let mut by_source = BTreeMap::new();
        for line in asc.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            let Some(at_s) = t.first().and_then(|s| s.parse::<f64>().ok()) else {
                continue;
            };
            // CAN FD: `t CANFD ch Rx id brs esi dlc len data..`;
            // classic: `t ch id Rx d dlc data..`.
            let (id, data) = match t.get(1) {
                Some(&"CANFD") => (t.get(4), t.get(9..)),
                _ => (t.get(2), t.get(6..)),
            };
            let (Some(&BATTERY_CAN_ID), Some(data)) = (id, data) else {
                continue;
            };
            let bytes: Option<Vec<u8>> = data.iter().take(4).map(|b| u8::from_str_radix(b, 16).ok()).collect();
            if let Some([b0, b1, b2, b3]) = bytes.as_deref() {
                let source = u64::from(u32::from_le_bytes([*b0, *b1, *b2, *b3]));
                by_source.entry(source).or_insert((at_s * 1000.0).round() as u64);
            }
        }
        Replay { by_source }
    }

    /// Replay time of the frame with this source TimeStamp.
    fn arrival(&self, source: u64) -> Option<u64> {
        self.by_source.get(&source).copied()
    }

    /// Replay time the frame with this source TimeStamp would have had
    /// without manipulation: the previous frame's replay time plus the source
    /// distance. Places injection windows; identity without frames.
    fn nominal(&self, source: u64) -> u64 {
        match self.by_source.range(..source).next_back() {
            Some((&prev, &at)) => at + (source - prev),
            None => source,
        }
    }

    /// Last source TimeStamp of the replay.
    fn source_end_ms(&self) -> Option<u64> {
        self.by_source.keys().next_back().copied()
    }
}

/// Replay length: timestamp of the last `.asc` frame, in ms. Frame lines start
/// with a timestamp; header and `End TriggerBlock` are skipped.
fn asc_end_ms(asc: &str) -> Result<u64> {
    let last_s = asc
        .lines()
        .filter_map(|l| l.split_whitespace().next()?.parse::<f64>().ok())
        .next_back()
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
    /// Observed strictly after the end of the stream the oracle covers
    /// (e.g. Guardian events during the collection drain): reported, but not
    /// verdict-relevant.
    PostHorizon,
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
            Status::Matched | Status::NotReached | Status::PostHorizon => false,
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
    /// Guardian events strictly after the end of the covered stream.
    post_horizon: usize,
}

/// Statistics-only view of the raw `GuardianEvidenceEvent` stream (ADR-007):
/// counts per detection class/level/state; never feeds the verdict.
#[derive(Serialize, Default)]
struct EvidenceStats {
    events: usize,
    /// Transitions without a causing sample, hence without `timestamp_ms`
    /// (ADR-017).
    without_source_timestamp: usize,
    /// `"CLASS/LEVEL stage" -> count`, e.g. `"PHYSICAL_TEMP_RATE/WARNING active"`.
    by_detection: BTreeMap<String, usize>,
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
    /// Raw detection transitions, counted only (ADR-007 raw evidence stream).
    evidence: EvidenceStats,
    /// OpenSOVD plane (absent with `--no-sovd`).
    sovd: Option<SovdReport>,
    notes: Vec<String>,
}

/// Matches each Guardian `Failed` change to one OpenSOVD activation of its
/// fault code within `[t - SOVD_EARLY_MS, t + SOVD_SLACK_MS]`.
fn match_sovd(
    faults: &[TimedFault],
    sovd: &SovdTimeline,
    url: &str,
) -> (SovdReport, Vec<Option<u64>>) {
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
                && a.t_ms
                    .is_some_and(|ta| ta + SOVD_EARLY_MS >= t && ta <= t + SOVD_SLACK_MS)
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
    replay: &'a Replay,
    replay_end_ms: u64,
    sovd_url: Option<&'a str>,
}

fn evaluate(case: &Case, messages: &[Message]) -> Report {
    let Timeline {
        faults: all_faults,
        battery_end,
        sovd,
        evidence,
    } = timeline(messages, case.replay);
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
        notes.push(format!(
            "{STALE_FAULT} at the end of the replay ignored ({stale_at_end} change(s))"
        ));
    }
    let faults = collapse_handovers(
        all_faults
            .iter()
            .filter(|f| !end_of_replay(f))
            .cloned()
            .collect(),
    );

    // Fault events strictly after the end of the stream the oracle covers
    // (`source_window_ms.end`, else the last `source_finished_at_ms` of the
    // ground truth) typically come from the collection drain after the replay:
    // reported, but not verdict-relevant. Without any such source the old
    // fail-closed behaviour stays in place.
    // Both timeline ends are source times; fault `t_ms` lives on the replay
    // timeline, so map the bound through the nominal replay mapping before
    // comparing (upstream `evaluate on the replay timeline` semantics).
    let stream_end = case
        .oracle
        .source_window_ms
        .map(|w| case.replay.nominal(w.end))
        .or_else(|| {
            case.injections
                .iter()
                .filter_map(|i| i.source_finished_at_ms.map(|f| case.replay.nominal(f)))
                .max()
        });
    let (faults, post_horizon_faults): (Vec<TimedFault>, Vec<TimedFault>) =
        faults.into_iter().partition(|f| match f.t_ms {
            Some(t) => stream_end.is_none_or(|e| t <= e),
            None => true,
        });

    match battery_end {
        None => {
            notes.push("no battery event received; fault events cannot be placed in time".into());
            verdict = Verdict::Inconclusive;
        }
        Some(end) if end + REPLAY_END_TOLERANCE_MS < case.replay_end_ms => {
            notes.push(format!(
                "battery stream ended at {end} ms, replay lasts {} ms",
                case.replay_end_ms
            ));
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
            notes.push(format!(
                "OpenSOVD not reachable at {} ({} failed polls)",
                r.url, r.errors
            ));
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
                && f.t_ms
                    .is_some_and(|t| e.at_ms <= t && t <= e.at_ms + GUARDIAN_SLACK_MS)
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
        notes.push(format!(
            "{unplaced} fault event(s) without time (before the battery stream)"
        ));
        verdict = verdict.max(Verdict::Inconclusive);
    }
    for (i, f) in faults
        .iter()
        .enumerate()
        .filter(|(i, f)| !used[*i] && f.t_ms.is_some())
    {
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
            sovd_missing: sovd_reachable
                && f.event.stage == Stage::Failed
                && visible_at[i].is_none(),
            injection_id: None,
        });
    }
    // Post-horizon events: in the report, not in the verdict.
    for f in &post_horizon_faults {
        transitions.push(TransitionResult {
            status: Status::PostHorizon,
            fault_id: f.event.fault_id.clone(),
            detection_class: f.event.detection_class.clone(),
            level: f.event.level.clone(),
            stage: f.event.stage,
            expected_at_ms: None,
            observed_at_ms: f.t_ms,
            signal: f.event.evidence.signal.clone(),
            sovd_visible_at_ms: None,
            sovd_missing: false,
            injection_id: None,
        });
    }
    transitions.sort_by_key(|t| t.time());

    // Injections own the changes from their start up to the next start.
    // Slots on the replay timeline, from where each injection's first frame
    // would have been sent without manipulation.
    let slot = |i: usize| {
        let start = case.replay.nominal(case.injections[i].window.0);
        (start, case.injections.get(i + 1).map(|n| case.replay.nominal(n.window.0)))
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
    // Raw evidence stream: statistics only, never part of the verdict.
    let mut evidence_stats = EvidenceStats::default();
    for e in &evidence {
        evidence_stats.events += 1;
        *evidence_stats
            .by_detection
            .entry(format!("{}/{}/{}", e.detection_class, e.level, e.stage))
            .or_default() += 1;
        if e.timestamp_ms.is_none() {
            evidence_stats.without_source_timestamp += 1;
        }
    }
    if evidence_stats.without_source_timestamp > 0 {
        notes.push(format!(
            "{} raw evidence event(s) without source timestamp",
            evidence_stats.without_source_timestamp
        ));
    }
    let summary = OracleSummary {
        file: case.oracle_file.to_owned(),
        allow_unspecified: allow,
        expected: expected.len(),
        not_applicable,
        matched: count(Status::Matched),
        missing: count(Status::Missing),
        not_reached: count(Status::NotReached),
        unexpected: count(Status::Unexpected),
        post_horizon: count(Status::PostHorizon),
    };
    if summary.missing > 0 {
        notes.push(format!(
            "{} expected Guardian change(s) missing",
            summary.missing
        ));
        verdict = Verdict::Fail;
    }
    if summary.unexpected > 0 {
        if allow {
            notes.push(format!(
                "{} unexpected Guardian change(s), allowed by the oracle",
                summary.unexpected
            ));
        } else {
            notes.push(format!(
                "{} unexpected Guardian change(s)",
                summary.unexpected
            ));
            verdict = Verdict::Fail;
        }
    }
    if summary.post_horizon > 0 {
        notes.push(format!(
            "{} post-horizon Guardian event(s) after stream end (reported, not verdict-relevant)",
            summary.post_horizon
        ));
    }
    if summary.not_reached > 0 {
        notes.push(format!(
            "{} expected change(s) after the end of the battery stream",
            summary.not_reached
        ));
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
        evidence: evidence_stats,
        sovd: sovd_report,
        notes,
    }
}

struct Args {
    prefix: String,
    oracle: Option<String>,
    report: Option<String>,
    fault_topic: String,
    evidence_topic: String,
    battery_topic: String,
    idle_timeout: Duration,
    sovd_url: Option<String>,
    /// Bind address of the live observer; `Some` when `--observer` is given.
    #[cfg(feature = "observer")]
    observer: Option<String>,
    /// Guardian model YAML used to derive the static bands.
    #[cfg(feature = "observer")]
    guardian_model: Option<String>,
    /// Optional path written with the frozen observer state as standalone HTML.
    #[cfg(feature = "observer")]
    observer_dump: Option<String>,
}

#[cfg(feature = "observer")]
const USAGE: &str = "usage: evidence_collector <prefix> [--oracle FILE] [--report FILE] \
[--fault-topic URI] [--evidence-topic URI] [--battery-topic URI] [--idle-timeout SECS] [--observer] \
[--observer-addr ADDR] [--guardian-model FILE] [--dump-html FILE] [--sovd-url URL | --no-sovd]";
#[cfg(not(feature = "observer"))]
const USAGE: &str = "usage: evidence_collector <prefix> [--oracle FILE] [--report FILE] \
[--fault-topic URI] [--evidence-topic URI] [--battery-topic URI] [--idle-timeout SECS] [--sovd-url URL | --no-sovd]";

fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let mut prefix = None;
    let (mut oracle, mut report) = (None, None);
    let mut fault_topic = uprotocol::DEFAULT_FAULT_TOPIC.to_owned();
    let mut evidence_topic = uprotocol::DEFAULT_EVIDENCE_TOPIC.to_owned();
    let mut battery_topic = uprotocol::DEFAULT_BATTERY_TOPIC.to_owned();
    let mut idle_timeout = Duration::from_secs(3);
    let mut sovd_url = Some(sovd::DEFAULT_URL.to_owned());
    #[cfg(feature = "observer")]
    let (mut observer, mut guardian_model, mut observer_dump) = (None, None, None);
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--oracle" => oracle = Some(value()?),
            "--report" => report = Some(value()?),
            "--fault-topic" => fault_topic = value()?,
            "--evidence-topic" => evidence_topic = value()?,
            "--battery-topic" => battery_topic = value()?,
            "--sovd-url" => sovd_url = Some(value()?),
            "--no-sovd" => sovd_url = None,
            "--idle-timeout" => {
                idle_timeout = Duration::from_secs_f64(value()?.parse().context("--idle-timeout")?)
            }
            #[cfg(feature = "observer")]
            "--observer" => observer = Some(observer::DEFAULT_ADDR.to_owned()),
            #[cfg(feature = "observer")]
            "--observer-addr" => observer = Some(value()?),
            #[cfg(feature = "observer")]
            "--guardian-model" => guardian_model = Some(value()?),
            #[cfg(feature = "observer")]
            "--dump-html" => observer_dump = Some(value()?),
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
        evidence_topic,
        battery_topic,
        idle_timeout,
        sovd_url,
        #[cfg(feature = "observer")]
        observer,
        #[cfg(feature = "observer")]
        guardian_model,
        #[cfg(feature = "observer")]
        observer_dump,
    })
}

fn print_report(report: &Report) {
    let end = report
        .battery_end_ms
        .map_or("-".into(), |e| format!("{e} ms"));
    println!(
        "{}: {:?} (battery timeline up to {end}, replay {} ms)",
        report.case, report.verdict, report.replay_end_ms
    );
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
            if r.sovd_missing > 0 {
                format!(", {} not in OpenSOVD", r.sovd_missing)
            } else {
                String::new()
            }
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
            Status::PostHorizon => "POST-HORIZON",
        };
        let sovd = match (&report.sovd, t.stage, t.sovd_visible_at_ms) {
            (Some(_), Stage::Failed, Some(at)) => format!("OpenSOVD {at} ms"),
            (Some(_), Stage::Failed, None) if t.status != Status::Missing => {
                "<- not in OpenSOVD".into()
            }
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
    let evidence = &report.evidence;
    if evidence.events > 0 {
        print!(
            "  raw evidence: {} transition(s), {} without source timestamp:",
            evidence.events, evidence.without_source_timestamp
        );
        for (detection, count) in &evidence.by_detection {
            print!(" {detection}={count}");
        }
        println!();
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

    let oracle_file = args
        .oracle
        .clone()
        .unwrap_or_else(|| format!("{}.oracle.yaml", args.prefix));
    let oracle = oracle::parse(
        &fs::read_to_string(&oracle_file).with_context(|| format!("reading {oracle_file}"))?,
    )
    .with_context(|| format!("parsing {oracle_file}"))?;

    let candidates = [
        format!("{}.ground_truth.yaml", args.prefix),
        format!("{}.json", args.prefix),
    ];
    let injections = match candidates.iter().find(|p| std::path::Path::new(p).exists()) {
        Some(file) => parse_ground_truth(
            &fs::read_to_string(file).with_context(|| format!("reading {file}"))?,
        )
        .with_context(|| format!("parsing {file}"))?,
        None => Vec::new(),
    };

    let replay_file = format!("{}.asc", args.prefix);
    let asc = fs::read_to_string(&replay_file).with_context(|| format!("reading {replay_file}"))?;
    let replay_end_ms = asc_end_ms(&asc).with_context(|| format!("parsing {replay_file}"))?;
    let replay = Replay::parse(&asc);
    eprintln!(
        "replay lasts {replay_end_ms} ms, {} injection(s), {} oracle transition(s)",
        injections.len(),
        oracle.transitions.len()
    );

    #[cfg(feature = "observer")]
    let observer = setup_observer(&args, &injections);
    #[cfg(feature = "observer")]
    let sink = observer.clone().and_then(observer::sink);
    #[cfg(not(feature = "observer"))]
    let sink: uprotocol::Sink = None;

    let topics = [&args.fault_topic, &args.evidence_topic, &args.battery_topic]
        .into_iter()
        .map(|t| UUri::from_str(t).with_context(|| format!("invalid topic {t}")))
        .collect::<Result<Vec<_>>>()?;
    let stop = uprotocol::StopCondition {
        // The listener sees source timestamps: wait for the last one.
        end_ms: replay.source_end_ms().unwrap_or(replay_end_ms),
        idle_timeout: args.idle_timeout,
    };
    let messages = uprotocol::collect(&topics, args.sovd_url.as_deref(), &stop, sink).await?;

    let report = evaluate(
        &Case {
            name: &args.prefix,
            oracle_file: &oracle_file,
            oracle: &oracle,
            injections: &injections,
            replay: &replay,
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

    // ADR-018: the frozen observer state is written as one self-contained HTML
    // document for the Evidence Reporter to link.
    #[cfg(feature = "observer")]
    if let (Some(dump), Some(handle)) = (&args.observer_dump, &observer) {
        match handle.dump_html(std::path::Path::new(dump)) {
            Ok(()) => println!("  observer html: {dump}"),
            Err(e) => eprintln!("observer: cannot write {dump}: {e}"),
        }
    }

    Ok(report.verdict)
}

#[cfg(feature = "observer")]
const DEFAULT_GUARDIAN_MODEL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../config/battery_guardian/guardian_model.yaml"
);

/// Builds the observer state, loads bands/ground truth/oracle, and spawns the
/// HTTP server. Returns `None` unless `--observer` was given.
#[cfg(feature = "observer")]
fn setup_observer(args: &Args, injections: &[Injection]) -> Option<observer::Handle> {
    let addr = args.observer.clone()?;
    let incidents = injections
        .iter()
        .map(|i| observer::Incident {
            injection_id: i.injection_id.clone(),
            injected_class: i.injected_class.clone(),
            start_ms: i.window.0,
            end_ms: i.window.1,
            mutations: i.mutations.clone(),
        })
        .collect();
    let run_id = injections.iter().find_map(|i| i.run_id.clone());
    let handle = observer::Observer::new(
        run_id,
        load_bands(args.guardian_model.as_deref()),
        incidents,
        load_oracle(&args.oracle, &args.prefix),
    );
    let server = handle.clone();
    tokio::spawn(async move {
        if let Err(e) = observer::serve(addr, server).await {
            eprintln!("observer: {e:#}");
        }
    });
    Some(handle)
}

/// Static bands from the authoritative Guardian model (ADR-005, ADR-016).
#[cfg(feature = "observer")]
fn load_bands(path: Option<&str>) -> Option<observer::Bands> {
    let env = std::env::var("GUARDIAN_MODEL").ok();
    let path = path.or(env.as_deref()).unwrap_or(DEFAULT_GUARDIAN_MODEL);
    match battery_guardian::GuardianConfig::load(path) {
        Ok(cfg) => Some(observer::Bands::from_config(&cfg)),
        Err(e) => {
            eprintln!("observer: cannot load model {path}: {e:#}");
            None
        }
    }
}

/// The experiment oracle, passed through as JSON for the expected lane.
/// Absent for baseline cases.
#[cfg(feature = "observer")]
fn load_oracle(oracle: &Option<String>, prefix: &str) -> Option<serde_json::Value> {
    let path = oracle
        .clone()
        .unwrap_or_else(|| format!("{prefix}.oracle.yaml"));
    let src = fs::read_to_string(&path).ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&src).ok()?;
    serde_json::to_value(value).ok()
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
            ..Default::default()
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
            evidence: FaultEvidence {
                signal: Some("temp_min".into()),
                timestamp_ms: ts,
                ..Default::default()
            },
        })
    }

    fn copy(m: &Message) -> Message {
        match m {
            Message::Fault(f) => Message::Fault(f.clone()),
            Message::Evidence(e) => Message::Evidence(e.clone()),
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
            out.extend(
                extra
                    .iter()
                    .filter(|(at, _)| *at == t)
                    .map(|(_, m)| copy(m)),
            );
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
    /// Injection window ending with the replay: without a
    /// `source_window_ms`, the last record finish doubles as the fallback
    /// stream end, so events up to `END` stay in the window.
    const GROUND_TRUTH: &str = "
- injection_id: oor-1
  injected_class: signal.out_of_range
  source_started_at_ms: 1000
  source_finished_at_ms: 19900
";
    const LIMIT: &str = "PHYSICAL_TEMP_ABSOLUTE_LIMIT";
    /// Oracle without transitions and without a source window.
    const EMPTY_ORACLE: &str = "guardian:\n  allow_unspecified: false\n  transitions: []\n";
    /// Oracle whose covered stream ends at 19_000 ms: Guardian events strictly
    /// after that are post-horizon.
    const WINDOWED_ORACLE: &str = "schema_version: 1
source_window_ms: { start: 0, end: 19000 }

guardian:
  allow_unspecified: false
  transitions: []
";

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
                replay: &Replay::default(),
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
        r.transitions
            .iter()
            .map(|t| (t.expected_at_ms, t.status))
            .collect()
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
        assert_eq!(
            check_with(&allowed, None, &stream(&extra)).verdict,
            Verdict::Pass
        );
    }

    #[test]
    fn empty_oracle_is_a_baseline() {
        assert_eq!(
            check_with(EMPTY_ORACLE, None, &stream(&[])).verdict,
            Verdict::Pass
        );
        let r = check_with(
            EMPTY_ORACLE,
            None,
            &stream(&[(500, fault(LIMIT, Stage::Failed, Some(500)))]),
        );
        assert_eq!(r.verdict, Verdict::Fail);
    }

    /// Identity replay fixture: source time == replay time (nominal template).
    fn identity_replay() -> Replay {
        let mut asc = String::from("date Wed Oct 07 2026\n");
        for s in (0..=20000).step_by(100) {
            asc += &frame(s, s);
            asc.push('\n');
        }
        Replay::parse(&asc)
    }

    /// Regression with a windowed oracle: everything up to (and at) the
    /// window end is judged as before, unexpected in-window changes fail.
    #[test]
    fn unexpected_in_window_fails() {
        let r = check_with(WINDOWED_ORACLE, None, &stream(&limit_ok()));
        assert_eq!(r.verdict, Verdict::Fail);
        assert_eq!(r.oracle.unexpected, 2);
        assert_eq!(r.oracle.post_horizon, 0);
    }

    /// Stream end from the oracle: Guardian events strictly after
    /// `source_window_ms.end` are post-horizon (reported, not
    /// verdict-relevant); an event at exactly the end stays in the window.
    #[test]
    fn post_horizon_after_the_window_is_not_verdict_relevant() {
        let r = check_with(
            WINDOWED_ORACLE,
            None,
            &stream(&[(19500, fault("SIGNAL_STUCK", Stage::Failed, Some(19500)))]),
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.notes);
        assert_eq!(r.oracle.unexpected, 0);
        assert_eq!(r.oracle.post_horizon, 1);
        let t = r
            .transitions
            .iter()
            .find(|t| t.status == Status::PostHorizon)
            .unwrap();
        assert_eq!(t.observed_at_ms, Some(19500));
        assert!(r.notes.iter().any(|n| n == "1 post-horizon Guardian event(s) after stream end (reported, not verdict-relevant)"));
        // The end edge itself belongs to the window.
        let edge = check_with(
            WINDOWED_ORACLE,
            None,
            &stream(&[(19000, fault("SIGNAL_STUCK", Stage::Failed, Some(19000)))]),
        );
        assert_eq!(edge.verdict, Verdict::Fail);
        assert_eq!(edge.oracle.unexpected, 1);
    }

    /// Fallback: without an oracle window, the last ground-truth finish marks
    /// the stream end.
    #[test]
    fn ground_truth_finish_is_the_fallback_stream_end() {
        let gt = "- injection_id: a
  injected_class: signal.out_of_range
  source_started_at_ms: 1000
  source_finished_at_ms: 19000
";
        let m = stream(&[(19500, fault("SIGNAL_STUCK", Stage::Failed, Some(19500)))]);
        let oracle = oracle::parse(EMPTY_ORACLE).unwrap();
        let injections = parse_ground_truth(gt).unwrap();
        let replay = identity_replay();
        let r = evaluate(
            &Case {
                name: "t",
                oracle_file: "o",
                oracle: &oracle,
                injections: &injections,
                replay: &replay,
replay_end_ms: END,
                sovd_url: None,
            },
            &m,
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.notes);
        assert_eq!(r.oracle.post_horizon, 1);
    }

    /// Without any stream-end source (no oracle window, no ground-truth
    /// finish) nothing changes: fail-closed, as before.
    #[test]
    fn without_a_stream_end_source_the_old_behaviour_stays() {
        let gt = "- injection_id: a
  injected_class: signal.out_of_range
  started_at: 1000
  duration_ms: 500
";
        let m = stream(&[(19500, fault("SIGNAL_STUCK", Stage::Failed, Some(19500)))]);
        let oracle = oracle::parse(EMPTY_ORACLE).unwrap();
        let injections = parse_ground_truth(gt).unwrap();
        assert!(injections[0].source_finished_at_ms.is_none());
        let replay = identity_replay();
        let r = evaluate(
            &Case {
                name: "t",
                oracle_file: "o",
                oracle: &oracle,
                injections: &injections,
                replay: &replay,
replay_end_ms: END,
                sovd_url: None,
            },
            &m,
        );
        assert_eq!(r.verdict, Verdict::Fail);
        assert_eq!(r.oracle.unexpected, 1);
        assert_eq!(r.oracle.post_horizon, 0);
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
        let m = stream_from(
            300,
            0,
            &[
                (1000, fault(LIMIT, Stage::Failed, None)),
                (1500, fault(LIMIT, Stage::Passed, None)),
            ],
        );
        let r = check(&m);
        assert_eq!(r.verdict, Verdict::Pass);
        assert_eq!(r.transitions[0].observed_at_ms, Some(1000));
    }

    #[test]
    fn wall_clock_timestamps_are_rebased() {
        let epoch = 1_791_300_000_000;
        let m = stream_from(
            0,
            epoch,
            &[
                (1000, fault(LIMIT, Stage::Failed, Some(epoch + 1000))),
                (1500, fault(LIMIT, Stage::Passed, None)),
            ],
        );
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
        m.insert(
            0,
            Message::Fault(FaultEvent {
                fault_id: "BatteryTempStreamStale".into(),
                detection_class: "STREAM_STALE".into(),
                level: "VIOLATION".into(),
                stage: Stage::Passed,
                baseline: true,
                evidence: FaultEvidence::default(),
            }),
        );
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
                replay: &Replay::default(),
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
                sovd::FaultState {
                    active,
                    occurrences,
                },
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
        let r = check_with(
            ORACLE,
            Some("http://sovd"),
            &with_sovd(vec![(1300, sovd("BatteryTempAbsoluteLimit", true, 1))]),
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.notes);
        assert_eq!(r.transitions[0].sovd_visible_at_ms, Some(1300));
    }

    #[test]
    fn sovd_too_late_or_missing_fails() {
        for extra in [
            vec![(1600, sovd("BatteryTempAbsoluteLimit", true, 1))],
            vec![],
        ] {
            let r = check_with(ORACLE, Some("http://sovd"), &with_sovd(extra));
            assert_eq!(r.verdict, Verdict::Fail);
            assert_eq!(r.injections[0].sovd_missing, 1);
            assert!(r.transitions[0].sovd_missing);
        }
    }

    #[test]
    fn sovd_short_fault_counted_by_occurrence_counter() {
        let r = check_with(
            ORACLE,
            Some("http://sovd"),
            &with_sovd(vec![(1100, sovd("BatteryTempAbsoluteLimit", false, 1))]),
        );
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
            m.push(Message::Battery(BatteryEvent {
                timestamp_ms: t,
                received: at(t),
                ..Default::default()
            }));
        }
        m.push(fault("STREAM_STALE", Stage::Failed, Some(8500)));
        let Message::Sovd(snapshot, _) = sovd("BatteryTempStreamStale", true, 1) else {
            unreachable!()
        };
        m.push(Message::Sovd(snapshot, at(8550)));
        for t in (8600..=END).step_by(100) {
            m.push(Message::Battery(BatteryEvent {
                timestamp_ms: t,
                received: at(t),
                ..Default::default()
            }));
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
    fn decodes_guardian_evidence_payload_and_counts_stats() {
        // A mapped active transition with evidence data and timestamp...
        let active =
            br#"{"run_id":"run-7","detection_class":"PHYSICAL_TEMP_RATE","level":"WARNING",
            "stage":"active","signal":"temp_min","evidence":{"observed":3.5},"timestamp_ms":1200}"#;
        let Message::Evidence(active_event) = decode_message(active).unwrap() else {
            panic!("not an evidence event");
        };
        assert_eq!(
            (active_event.run_id.as_str(), active_event.stage.as_str()),
            ("run-7", "active")
        );
        assert_eq!(
            (active_event.level.as_str(), active_event.signal.as_deref()),
            ("WARNING", Some("temp_min"))
        );
        assert_eq!(active_event.timestamp_ms, Some(1200));
        // ...and an unmapped cleared transition without any causing sample.
        let cleared = br#"{"run_id":"run-7","detection_class":"STREAM_STALE","level":"VIOLATION","stage":"cleared"}"#;
        let Message::Evidence(cleared_event) = decode_message(cleared).unwrap() else {
            panic!("not an evidence event");
        };
        assert_eq!(cleared_event.timestamp_ms, None);
        assert_eq!(cleared_event.stage, "cleared");

        // Raw events are collected for statistics, not judged.
        let replay = identity_replay();
        let timeline = timeline(
            &[
                Message::Evidence(active_event.clone()),
                battery(100),
                Message::Evidence(cleared_event.clone()),
            ],
            &replay,
        );
        assert!(timeline.faults.is_empty());
        assert_eq!(timeline.evidence.len(), 2);
    }

    #[test]
    fn decodes_guardian_and_battery_payloads() {
        let fault = br#"{"fault_id":"BatteryTempRate","detection_class":"PHYSICAL_TEMP_RATE",
            "level":"VIOLATION","stage":"Failed","baseline":false,"sovd_path":"battery_guardian",
            "source":{"entity":"BatteryThermalGuardian"},"evidence":{"signal":"temp_max","timestamp_ms":1200}}"#;
        let Message::Fault(f) = decode_message(fault).unwrap() else {
            panic!("not a fault")
        };
        assert_eq!(
            (f.stage, f.evidence.timestamp_ms),
            (Stage::Failed, Some(1200))
        );
        let baseline = br#"{"fault_id":"BatteryTempRate","detection_class":"PHYSICAL_TEMP_RATE",
            "level":"VIOLATION","stage":"Passed","baseline":true,"evidence":{}}"#;
        assert!(matches!(decode_message(baseline).unwrap(), Message::Fault(f) if f.baseline));
        let battery =
            br#"{"temp_max":32.5,"temp_avg":30.5,"temp_min":25.5,"soc":67.0,"timestamp_ms":1200}"#;
        assert!(
            matches!(decode_message(battery).unwrap(), Message::Battery(b) if b.timestamp_ms == 1200)
        );
    }

    /// Frame line as the case mutator writes it (CAN FD, TimeStamp LE).
    fn frame(at_ms: u64, source: u64) -> String {
        let b = (source as u32).to_le_bytes();
        format!(
            "{:.6} CANFD 1 Rx 100 0 0 a 16 {:02X} {:02X} {:02X} {:02X} 8C 00 8F 00 87 00 5A 00 00 00 00 00",
            at_ms as f64 / 1000.0,
            b[0], b[1], b[2], b[3]
        )
    }

    #[test]
    fn replay_maps_source_timestamps_to_replay_time() {
        // Frames 0..7900 on time; 8000 held back by 700 ms, later ones too.
        let mut asc = String::from("date Wed Oct 07 2026\nbase hex  timestamps absolute\n");
        for s in (0..=7900).step_by(100) {
            asc += &frame(s, s);
            asc.push('\n');
        }
        for s in (8000..=9000).step_by(100) {
            asc += &frame(s + 700, s);
            asc.push('\n');
        }
        let r = Replay::parse(&asc);
        assert_eq!(r.arrival(7900), Some(7900));
        assert_eq!(r.arrival(8000), Some(8700));
        assert_eq!(r.nominal(8000), 8000);
        assert_eq!(r.source_end_ms(), Some(9000));
        assert_eq!(asc_end_ms(&asc).unwrap(), 9700);
    }

    /// transport.delay: stale during the hold-back and its clear when the
    /// delayed frame arrives, both on the replay timeline like the oracle.
    #[test]
    fn delayed_frames_are_evaluated_on_the_replay_timeline() {
        let mut asc = String::new();
        for s in (0..=7900).step_by(100) {
            asc += &frame(s, s);
            asc.push('\n');
        }
        for s in (8000..=END).step_by(100) {
            asc += &frame(s + 700, s);
            asc.push('\n');
        }
        let replay = Replay::parse(&asc);
        let oracle = oracle::parse(
            "guardian:
  allow_unspecified: false
  transitions:
    - { at_ms: 8500, class: STREAM_STALE, level: VIOLATION, state: active }
    - { at_ms: 8700, class: STREAM_STALE, level: VIOLATION, state: cleared }
",
        )
        .unwrap();
        let gt = "- { injection_id: d, injected_class: transport.delay, source_started_at_ms: 8000, source_finished_at_ms: 8100 }";
        let injections = parse_ground_truth(gt).unwrap();
        let mut m = Vec::new();
        for s in (0..=7900).step_by(100) {
            m.push(battery(s));
        }
        // Guardian: stale projected to 7900 + 600 on the source time.
        m.push(fault("STREAM_STALE", Stage::Failed, Some(8500)));
        m.push(battery(8000));
        m.push(fault("STREAM_STALE", Stage::Passed, Some(8000)));
        for s in (8100..=END).step_by(100) {
            m.push(battery(s));
        }
        let r = evaluate(
            &Case {
                name: "t",
                oracle_file: "o",
                oracle: &oracle,
                injections: &injections,
                replay: &replay,
                replay_end_ms: END + 700,
                sovd_url: None,
            },
            &m,
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?} {:?}", r.notes, r.transitions);
        assert_eq!(r.battery_end_ms, Some(END + 700));
        assert_eq!(r.injections[0].matched, 2);
    }

    #[test]
    fn golden_asc_lasts_19900_ms() {
        let asc = include_str!("../../../tests/battery_campaign/scenarios/0_cold_nominal.asc");
        assert_eq!(asc_end_ms(asc).unwrap(), 19_900);
    }
}
