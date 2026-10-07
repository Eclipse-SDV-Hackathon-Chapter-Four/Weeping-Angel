//! Evidence collector.
//!
//! Started with a case prefix: reads the ground-truth record
//! `<prefix>.ground_truth.yaml` (case mutator output; `<prefix>.json` as
//! fallback, e.g. `[]` for a baseline) and the replay `<prefix>.asc`, then
//! listens on uProtocol to
//! - `GuardianFaultEvent` (`//guardian/1001/1/8001`): fault-level changes, and
//! - `BatteryTempEvent` (`//battery-vss/9001/1/9001`): the battery stream.
//!
//! Fault events carry no time, so each one is placed on the battery source
//! timeline (ADR-008): it gets the `timestamp_ms` of the latest battery event,
//! relative to the first battery event received.
//!
//! An injection passes if a non-baseline `Failed` event of an expected class
//! (`expected_observations.yaml`) arrives within
//! `source_started_at_ms <= t <= source_finished_at_ms`, both in ms on that
//! timeline (hand-written records may use a numeric `started_at` with
//! `finished_at` or `duration_ms` instead; the mutator's epoch `started_at` is
//! informational). Other failures, before or after, are allowed and reported.
//! A case without injections (baseline) passes only without any failure.
//!
//! Collection stops once the battery timeline reaches the end of the replay
//! (last `.asc` frame) plus `--idle-timeout` for late events, or after
//! `--idle-timeout` without any message.
//!
//! Usage:
//!   evidence_collector <prefix> [--fault-topic URI] [--battery-topic URI]
//!       [--idle-timeout SECS] [--expectations FILE] [--report FILE]
//!
//! The JSON report goes to `reports/<case name>.json` (relative to the current
//! directory) unless `--report` names another file.
//! `ZENOH_CONNECT` selects the Zenoh router.
//! Exit code: 0 PASS, 1 FAIL, 2 INCONCLUSIVE, 3 usage/input error.

mod uprotocol;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use up_rust::UUri;

#[cfg(test)]
const CONTRACT: &str = include_str!("../../../interfaces/battery_fault_contract.yaml");
const DEFAULT_EXPECTATIONS: &str = include_str!("../expected_observations.yaml");

/// Slack when comparing the battery timeline with the replay end, to absorb
/// clock jitter when the bridge still sends wall-clock timestamps.
const REPLAY_END_TOLERANCE_MS: u64 = 200;

/// Injected class -> Guardian detection classes that count as its detection.
type Expectations = BTreeMap<String, BTreeSet<String>>;

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Failed,
    Passed,
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
}

/// `BatteryTempEvent`; only the source timestamp is needed.
#[derive(Deserialize, Clone, Debug)]
struct BatteryEvent {
    timestamp_ms: u64,
}

#[derive(Debug)]
enum Message {
    Fault(FaultEvent),
    Battery(BatteryEvent),
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

/// One ground-truth record (contract `campaign_ground_truth_event`).
#[derive(Deserialize, Serialize, Clone, Debug)]
struct Injection {
    #[serde(default)]
    run_id: Option<String>,
    injection_id: String,
    injected_class: String,
    /// Injection window in ms on the battery source timeline (case mutator).
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

/// Parses a ground-truth file: one record, a list of records, or empty/`[]`
/// for a baseline case. JSON and YAML are both accepted.
fn parse_ground_truth(src: &str) -> Result<Vec<Injection>> {
    let value: serde_yaml::Value = serde_yaml::from_str(src)?;
    let records = match value {
        serde_yaml::Value::Null => Vec::new(),
        serde_yaml::Value::Sequence(items) => items,
        other => vec![other],
    };
    records
        .into_iter()
        .map(|r| {
            let mut inj: Injection =
                serde_yaml::from_value(r).context("invalid ground-truth record")?;
            inj.window = inj.source_window()?;
            Ok(inj)
        })
        .collect()
}

/// A fault event placed on the battery timeline. `t_ms` is `None` if it
/// arrived before the first battery event.
#[derive(Serialize, Clone, Debug)]
struct TimedFault {
    t_ms: Option<u64>,
    #[serde(flatten)]
    event: FaultEvent,
}

/// Places fault events on the battery timeline; also returns the last
/// battery position (`None` if no battery event arrived).
fn timeline(messages: &[Message]) -> (Vec<TimedFault>, Option<u64>) {
    let mut origin = None;
    let mut now = None;
    let mut faults = Vec::new();
    for m in messages {
        match m {
            Message::Battery(b) => {
                let o = *origin.get_or_insert(b.timestamp_ms);
                now = Some(b.timestamp_ms.saturating_sub(o));
            }
            Message::Fault(f) => faults.push(TimedFault {
                t_ms: now,
                event: f.clone(),
            }),
        }
    }
    (faults, now)
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

#[derive(Serialize)]
struct InjectionResult {
    #[serde(flatten)]
    injection: Injection,
    window_ms: (u64, u64),
    expected_classes: BTreeSet<String>,
    verdict: Verdict,
    detected_class: Option<String>,
    detected_at_ms: Option<u64>,
    latency_ms: Option<u64>,
}

#[derive(Serialize)]
struct Report {
    case: String,
    verdict: Verdict,
    replay_end_ms: u64,
    battery_end_ms: Option<u64>,
    injections: Vec<InjectionResult>,
    /// All non-baseline fault events, in arrival order.
    fault_events: Vec<TimedFault>,
    notes: Vec<String>,
}

fn evaluate(
    case: &str,
    injections: &[Injection],
    expectations: &Expectations,
    replay_end_ms: u64,
    messages: &[Message],
) -> Report {
    let (faults, battery_end) = timeline(messages);
    let faults: Vec<TimedFault> = faults.into_iter().filter(|f| !f.event.baseline).collect();
    let failures = || faults.iter().filter(|f| f.event.stage == Stage::Failed);
    let mut notes = Vec::new();
    let mut verdict = Verdict::Pass;

    let replay_complete =
        battery_end.is_some_and(|end| end + REPLAY_END_TOLERANCE_MS >= replay_end_ms);
    match battery_end {
        None => {
            notes.push("no battery event received; fault events cannot be placed in time".into());
            verdict = Verdict::Inconclusive;
        }
        Some(end) if !replay_complete => {
            notes.push(format!("battery stream ended at {end} ms, replay lasts {replay_end_ms} ms"));
            verdict = Verdict::Inconclusive;
        }
        _ => {}
    }

    if injections.is_empty() && battery_end.is_some() {
        let count = failures().count();
        if count > 0 {
            notes.push(format!("baseline case, but {count} failure(s) reported"));
            verdict = Verdict::Fail;
        }
    }

    let results = injections
        .iter()
        .map(|inj| {
            let (start, finish) = inj.window;
            let expected = expectations.get(&inj.injected_class).cloned().unwrap_or_default();
            let hit = failures().find(|f| {
                expected.contains(&f.event.detection_class)
                    && f.t_ms.is_some_and(|t| start <= t && t <= finish)
            });
            let result = if expected.is_empty() {
                notes.push(format!("{}: no expected class defined for {}", inj.injection_id, inj.injected_class));
                Verdict::Inconclusive
            } else if hit.is_some() {
                Verdict::Pass
            } else if battery_end.is_none_or(|end| end < finish) {
                notes.push(format!("{}: stream ended before its window closed", inj.injection_id));
                Verdict::Inconclusive
            } else {
                notes.push(format!(
                    "{}: no {:?} failure within [{start}, {finish}] ms",
                    inj.injection_id, expected
                ));
                Verdict::Fail
            };
            verdict = verdict.max(result);
            InjectionResult {
                injection: inj.clone(),
                window_ms: (start, finish),
                expected_classes: expected,
                verdict: result,
                detected_class: hit.map(|f| f.event.detection_class.clone()),
                detected_at_ms: hit.and_then(|f| f.t_ms),
                latency_ms: hit.and_then(|f| f.t_ms).map(|t| t - start),
            }
        })
        .collect();

    Report {
        case: case.to_owned(),
        verdict,
        replay_end_ms,
        battery_end_ms: battery_end,
        injections: results,
        fault_events: faults,
        notes,
    }
}

struct Args {
    prefix: String,
    expectations: Option<String>,
    report: Option<String>,
    fault_topic: String,
    battery_topic: String,
    idle_timeout: Duration,
}

const USAGE: &str = "usage: evidence_collector <prefix> [--fault-topic URI] [--battery-topic URI] \
[--idle-timeout SECS] [--expectations FILE] [--report FILE]";

fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let mut prefix = None;
    let (mut expectations, mut report) = (None, None);
    let mut fault_topic = uprotocol::DEFAULT_FAULT_TOPIC.to_owned();
    let mut battery_topic = uprotocol::DEFAULT_BATTERY_TOPIC.to_owned();
    let mut idle_timeout = Duration::from_secs(3);
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--expectations" => expectations = Some(value()?),
            "--report" => report = Some(value()?),
            "--fault-topic" => fault_topic = value()?,
            "--battery-topic" => battery_topic = value()?,
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
        expectations,
        report,
        fault_topic,
        battery_topic,
        idle_timeout,
    })
}

async fn run() -> Result<Verdict> {
    let args = parse_args()?;

    let expectations_src = match &args.expectations {
        Some(path) => fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
        None => DEFAULT_EXPECTATIONS.to_owned(),
    };
    let expectations: Expectations =
        serde_yaml::from_str(&expectations_src).context("parsing expectations")?;

    let candidates = [
        format!("{}.ground_truth.yaml", args.prefix),
        format!("{}.json", args.prefix),
    ];
    let sidecar = candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .with_context(|| format!("no ground truth: neither {} nor {}", candidates[0], candidates[1]))?;
    let injections = parse_ground_truth(
        &fs::read_to_string(&sidecar).with_context(|| format!("reading {sidecar}"))?,
    )
    .with_context(|| format!("parsing {sidecar}"))?;
    for inj in &injections {
        if !expectations.contains_key(&inj.injected_class) {
            eprintln!("warning: {} not listed in expectations", inj.injected_class);
        }
    }

    let replay = format!("{}.asc", args.prefix);
    let replay_end_ms = asc_end_ms(&fs::read_to_string(&replay).with_context(|| format!("reading {replay}"))?)
        .with_context(|| format!("parsing {replay}"))?;
    eprintln!("replay lasts {replay_end_ms} ms, {} injection(s)", injections.len());

    let topics = [&args.fault_topic, &args.battery_topic]
        .into_iter()
        .map(|t| UUri::from_str(t).with_context(|| format!("invalid topic {t}")))
        .collect::<Result<Vec<_>>>()?;
    let stop = uprotocol::StopCondition {
        end_ms: replay_end_ms,
        idle_timeout: args.idle_timeout,
    };
    let messages = uprotocol::collect(&topics, &stop).await?;

    let report = evaluate(&args.prefix, &injections, &expectations, replay_end_ms, &messages);

    let end = report.battery_end_ms.map_or("-".into(), |e| format!("{e} ms"));
    println!("{}: {:?} (battery timeline up to {end}, replay {} ms)", report.case, report.verdict, report.replay_end_ms);
    for r in &report.injections {
        let (s, f) = r.window_ms;
        match (&r.detected_class, r.latency_ms) {
            (Some(class), Some(latency)) => println!(
                "  {} ({}) [{s}, {f}] ms -> {class} after {latency} ms",
                r.injection.injection_id, r.injection.injected_class
            ),
            _ => println!(
                "  {} ({}) [{s}, {f}] ms -> {:?}",
                r.injection.injection_id, r.injection.injected_class, r.verdict
            ),
        }
    }
    let failures: Vec<_> = report.fault_events.iter().filter(|f| f.event.stage == Stage::Failed).collect();
    println!("  {} failure event(s) in total", failures.len());
    for f in &failures {
        let t = f.t_ms.map_or("before stream".into(), |t| format!("{t} ms"));
        println!("    {t}: {} / {} ({})", f.event.detection_class, f.event.level, f.event.fault_id);
    }
    for note in &report.notes {
        println!("  note: {note}");
    }

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

    const END: u64 = 13_900;

    fn expectations() -> Expectations {
        serde_yaml::from_str(DEFAULT_EXPECTATIONS).unwrap()
    }

    fn battery(t: u64) -> Message {
        Message::Battery(BatteryEvent { timestamp_ms: t })
    }

    fn fault(class: &str, stage: Stage) -> Message {
        Message::Fault(FaultEvent {
            fault_id: format!("Fault{class}"),
            detection_class: class.into(),
            level: "VIOLATION".into(),
            stage,
            baseline: false,
        })
    }

    /// Battery events every 100 ms up to `END`, with `extra` messages inserted
    /// right after the battery event at the given time.
    fn stream(origin: u64, extra: &[(u64, Message)]) -> Vec<Message> {
        let mut out = Vec::new();
        for t in (0..=END).step_by(100) {
            out.push(battery(origin + t));
            for (at, m) in extra {
                if *at == t {
                    out.push(match m {
                        Message::Fault(f) => Message::Fault(f.clone()),
                        Message::Battery(b) => Message::Battery(b.clone()),
                    });
                }
            }
        }
        out
    }

    fn check(ground_truth: &str, messages: &[Message]) -> Report {
        let injections = parse_ground_truth(ground_truth).unwrap();
        evaluate("test", &injections, &expectations(), END, messages)
    }

    const STUCK: &str = r#"{"injection_id": "signal_stuck", "injected_class": "signal.stuck",
                           "started_at": 5000, "duration_ms": 2000}"#;

    #[test]
    fn baseline_without_failures_passes() {
        assert_eq!(check("[]", &stream(0, &[])).verdict, Verdict::Pass);
    }

    #[test]
    fn baseline_with_failure_fails() {
        let r = check("[]", &stream(0, &[(5000, fault("SIGNAL_STUCK", Stage::Failed))]));
        assert_eq!(r.verdict, Verdict::Fail);
    }

    #[test]
    fn baseline_events_are_ignored() {
        let mut m = stream(0, &[]);
        m.insert(0, Message::Fault(FaultEvent {
            fault_id: "BatteryTempStreamStale".into(),
            detection_class: "STREAM_STALE".into(),
            level: "VIOLATION".into(),
            stage: Stage::Passed,
            baseline: true,
        }));
        assert_eq!(check("[]", &m).verdict, Verdict::Pass);
    }

    #[test]
    fn detection_inside_window_passes_with_latency() {
        let r = check(STUCK, &stream(0, &[(5300, fault("SIGNAL_STUCK", Stage::Failed))]));
        assert_eq!(r.verdict, Verdict::Pass);
        assert_eq!(r.injections[0].latency_ms, Some(300));
    }

    #[test]
    fn window_bounds_are_inclusive() {
        for at in [5000, 7000] {
            let r = check(STUCK, &stream(0, &[(at, fault("SIGNAL_STUCK", Stage::Failed))]));
            assert_eq!(r.verdict, Verdict::Pass, "detection at {at} ms");
        }
    }

    #[test]
    fn detection_after_window_fails() {
        let r = check(STUCK, &stream(0, &[(7100, fault("SIGNAL_STUCK", Stage::Failed))]));
        assert_eq!(r.verdict, Verdict::Fail);
    }

    #[test]
    fn failures_outside_window_are_allowed() {
        let r = check(
            STUCK,
            &stream(0, &[
                (1000, fault("STREAM_STALE", Stage::Failed)),
                (5500, fault("SIGNAL_STUCK", Stage::Failed)),
                (9000, fault("PHYSICAL_TEMP_RATE", Stage::Failed)),
            ]),
        );
        assert_eq!(r.verdict, Verdict::Pass);
        assert_eq!(r.fault_events.len(), 3);
    }

    #[test]
    fn wrong_class_in_window_fails() {
        let r = check(STUCK, &stream(0, &[(5500, fault("STREAM_STALE", Stage::Failed))]));
        assert_eq!(r.verdict, Verdict::Fail);
    }

    #[test]
    fn passed_stage_is_not_a_detection() {
        let r = check(STUCK, &stream(0, &[(5500, fault("SIGNAL_STUCK", Stage::Passed))]));
        assert_eq!(r.verdict, Verdict::Fail);
    }

    #[test]
    fn wall_clock_battery_timestamps_are_rebased() {
        let r = check(STUCK, &stream(1_791_300_000_000, &[(5300, fault("SIGNAL_STUCK", Stage::Failed))]));
        assert_eq!(r.verdict, Verdict::Pass);
        assert_eq!(r.injections[0].detected_at_ms, Some(5300));
    }

    #[test]
    fn short_stream_is_inconclusive() {
        let m: Vec<_> = stream(0, &[]).into_iter().take(50).collect();
        assert_eq!(check("[]", &m).verdict, Verdict::Inconclusive);
        assert_eq!(check(STUCK, &m).verdict, Verdict::Inconclusive);
    }

    #[test]
    fn no_battery_events_is_inconclusive() {
        let r = check(STUCK, &[fault("SIGNAL_STUCK", Stage::Failed)]);
        assert_eq!(r.verdict, Verdict::Inconclusive);
        assert_eq!(r.fault_events[0].t_ms, None);
    }

    #[test]
    fn combination_has_no_expectation() {
        let gt = r#"{"injection_id": "c", "injected_class": "signal.combination",
                     "started_at": 1000, "duration_ms": 2000}"#;
        assert_eq!(check(gt, &stream(0, &[])).verdict, Verdict::Inconclusive);
    }

    #[test]
    fn ground_truth_formats() {
        assert!(parse_ground_truth("[]").unwrap().is_empty());
        assert!(parse_ground_truth("").unwrap().is_empty());
        assert_eq!(parse_ground_truth(STUCK).unwrap()[0].window, (5000, 7000));
        let yaml = "injection_id: s\ninjected_class: signal.spike\nstarted_at: 100\nfinished_at: 200\n";
        assert_eq!(parse_ground_truth(yaml).unwrap()[0].window, (100, 200));
        let epoch_only = r#"{"injection_id": "s", "injected_class": "signal.spike",
                             "started_at": "2026-10-07T12:00:00Z", "duration_ms": 100}"#;
        assert!(parse_ground_truth(epoch_only).is_err());
        let untimed = r#"{"injection_id": "s", "injected_class": "signal.spike", "started_at": 1}"#;
        assert!(parse_ground_truth(untimed).is_err());
    }

    /// Record as written by the case mutator (`<stem>.ground_truth.yaml`).
    #[test]
    fn case_mutator_ground_truth_uses_source_window() {
        let yaml = "\
run_id: r1
injection_id: signal_out_of_range
injected_class: signal.out_of_range
started_at: 2026-10-07T12:00:00.000Z
duration_ms: 1000
source_started_at_ms: 2000
source_finished_at_ms: 3000
battery_model:
  path: product/config/battery_guardian/guardian_model.yaml
  sha256: sha256:abc
mutations:
- signal: temp_min
  operator: out_of_range
  requested_parameters:
    value: -35.0
    duration_samples: 10
  executed_values: [-35.0]
";
        let inj = &parse_ground_truth(yaml).unwrap()[0];
        assert_eq!(inj.window, (2000, 3000));
        let r = evaluate(
            "test",
            std::slice::from_ref(inj),
            &expectations(),
            END,
            &stream(0, &[(2500, fault("PHYSICAL_TEMP_ABSOLUTE_LIMIT", Stage::Failed))]),
        );
        assert_eq!(r.verdict, Verdict::Pass);
        assert_eq!(r.injections[0].latency_ms, Some(500));
    }

    #[test]
    fn decodes_guardian_and_battery_payloads() {
        let fault = br#"{"fault_id":"BatteryTempRate","detection_class":"PHYSICAL_TEMP_RATE",
            "level":"VIOLATION","stage":"Failed","baseline":false,"sovd_path":"battery_guardian",
            "source":{"entity":"BatteryThermalGuardian"},"evidence":{"signal":"temp_max"}}"#;
        assert!(matches!(decode_message(fault).unwrap(), Message::Fault(f) if f.stage == Stage::Failed));
        let battery = br#"{"temp_max":32.5,"temp_avg":30.5,"temp_min":25.5,"soc":67.0,"timestamp_ms":1200}"#;
        assert!(matches!(decode_message(battery).unwrap(), Message::Battery(b) if b.timestamp_ms == 1200));
    }

    #[test]
    fn every_injection_class_has_expectations() {
        let model: serde_yaml::Value = serde_yaml::from_str(include_str!(
            "../../../config/battery_guardian/fault_injection_model.yaml"
        ))
        .unwrap();
        let e = expectations();
        for inj in model["injections"].as_sequence().unwrap() {
            let class = inj["injected_class"].as_str().unwrap();
            assert!(e.contains_key(class), "{class} missing in expected_observations.yaml");
        }
    }

    #[test]
    fn baseline_asc_lasts_13900_ms() {
        let asc = include_str!("../../../config/battery_temp_with_ts.asc");
        assert_eq!(asc_end_ms(asc).unwrap(), 13_900);
    }
}
