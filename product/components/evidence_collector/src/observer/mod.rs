//! Live Scenario Observer (ADR-016).
//!
//! A read-only, feature-flagged module of the Evidence Collector. It keeps a
//! 20 s ring buffer of battery samples, the detected Guardian fault classes,
//! and the run's ground truth/oracle, and serves them to a static frontend
//! over HTTP + Server-Sent Events. It reads in-process collector state only:
//! there is no separate UI process and no collector-to-UI transport.
//!
//! Contract: `product/doc/observer/live_observer.md`.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::{BatteryEvent, FaultEvent, FaultEvidence, Message, Stage};

/// Default bind address. Binds all interfaces so the observer is reachable
/// from outside the host; port 8080 is the Guardian. Override with
/// `--observer-addr` to restrict the listener to a loopback address.
pub(crate) const DEFAULT_ADDR: &str = "0.0.0.0:8090";
/// Sliding window kept in the ring buffer, matching the 20 s run convention.
pub(crate) const WINDOW_MS: u64 = 20_000;
const MAX_DETECTIONS: usize = 512;
const BROADCAST_CAPACITY: usize = 1024;

/// Static model bands derived from `guardian_model.yaml` through the
/// authoritative `battery-guardian` library (ADR-005, ADR-016).
#[derive(Serialize, Clone, Debug)]
pub(crate) struct Bands {
    pub temp_abs_min_c: f32,
    pub temp_warning_c: f32,
    pub temp_abs_max_c: f32,
    pub soc_min: f32,
    pub soc_max: f32,
}

impl Bands {
    pub(crate) fn from_config(cfg: &battery_guardian::GuardianConfig) -> Self {
        Self {
            temp_abs_min_c: cfg.temperature.absolute_min_c,
            temp_warning_c: cfg.temperature.warning_threshold_c(),
            temp_abs_max_c: cfg.temperature.critical_threshold_c(),
            soc_min: cfg.soc.min_percent,
            soc_max: cfg.soc.max_percent,
        }
    }
}

/// One battery sample on the relative timeline.
#[derive(Serialize, Clone, Debug)]
pub(crate) struct Sample {
    pub timestamp_ms: u64,
    pub temp_min: Option<f64>,
    pub temp_avg: Option<f64>,
    pub temp_max: Option<f64>,
    pub soc: Option<f64>,
}

/// One Guardian fault event placed on the relative timeline.
#[derive(Serialize, Clone, Debug)]
pub(crate) struct Detection {
    pub at_ms: u64,
    pub detection_class: String,
    pub level: String,
    pub stage: String,
    pub baseline: bool,
    pub fault_id: String,
    pub evidence: FaultEvidence,
}

/// One injection incident from the experiment ground truth.
#[derive(Serialize, Clone, Debug)]
pub(crate) struct Incident {
    pub injection_id: String,
    pub injected_class: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub mutations: Vec<serde_yaml::Value>,
}

/// A broadcast delta with a sequence number for snapshot/delta de-duplication.
#[derive(Clone)]
struct LiveEvent {
    name: &'static str,
    seq: u64,
    json: String,
}

struct Inner {
    seq: u64,
    run_id: Option<String>,
    t0_ms: Option<u64>,
    bands: Option<Bands>,
    samples: VecDeque<Sample>,
    detections: Vec<Detection>,
    incidents: Vec<Incident>,
    oracle: Option<serde_json::Value>,
}

impl Inner {
    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
}

/// Shared observer state.
pub(crate) struct Observer {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<LiveEvent>,
}

/// Cloneable handle to the observer state.
pub(crate) type Handle = Arc<Observer>;

impl Observer {
    pub(crate) fn new(
        run_id: Option<String>,
        bands: Option<Bands>,
        incidents: Vec<Incident>,
        oracle: Option<serde_json::Value>,
    ) -> Handle {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Arc::new(Self {
            inner: Mutex::new(Inner {
                seq: 0,
                run_id,
                t0_ms: None,
                bands,
                samples: VecDeque::new(),
                detections: Vec::new(),
                incidents,
                oracle,
            }),
            tx,
        })
    }

    /// Records one live message; called from the subscription loop.
    pub(crate) fn record_message(&self, message: &Message) {
        match message {
            Message::Battery(b) => self.record_sample(b),
            Message::Fault(f) => self.record_fault(f),
            // Raw evidence events are not part of the observer v1 view:
            // the mapped fault stream already covers the observer timeline.
            Message::Evidence(..) => {}
            // OpenSOVD findings are not part of the observer v1 view (ADR-016).
            Message::Sovd(..) => {}
        }
    }

    fn record_sample(&self, b: &BatteryEvent) {
        let mut inner = self.inner.lock().expect("observer mutex");
        let t0 = *inner.t0_ms.get_or_insert(b.timestamp_ms);
        let t = b.timestamp_ms.saturating_sub(t0);
        let sample = Sample {
            timestamp_ms: t,
            temp_min: b.temp_min,
            temp_avg: b.temp_avg,
            temp_max: b.temp_max,
            soc: b.soc,
        };
        let cutoff = t.saturating_sub(WINDOW_MS);
        while inner
            .samples
            .front()
            .is_some_and(|s| s.timestamp_ms < cutoff)
        {
            inner.samples.pop_front();
        }
        inner.samples.push_back(sample.clone());
        let seq = inner.bump();
        let json = serde_json::to_string(&sample).unwrap_or_default();
        drop(inner);
        let _ = self.tx.send(LiveEvent {
            name: "sample",
            seq,
            json,
        });
    }

    fn record_fault(&self, f: &FaultEvent) {
        if f.baseline {
            return; // synthetic DFM baseline, not an original decision (ADR-007)
        }
        let mut inner = self.inner.lock().expect("observer mutex");
        let at_ms = match (f.evidence.timestamp_ms, inner.t0_ms) {
            (Some(ts), Some(t0)) => ts.saturating_sub(t0),
            _ => inner.samples.back().map_or(0, |s| s.timestamp_ms),
        };
        let detection = Detection {
            at_ms,
            detection_class: f.detection_class.clone(),
            level: f.level.clone(),
            stage: match f.stage {
                Stage::Failed => "Failed",
                Stage::Passed => "Passed",
            }
            .to_owned(),
            baseline: f.baseline,
            fault_id: f.fault_id.clone(),
            evidence: f.evidence.clone(),
        };
        inner.detections.push(detection.clone());
        if inner.detections.len() > MAX_DETECTIONS {
            inner.detections.remove(0);
        }
        let seq = inner.bump();
        let json = serde_json::to_string(&detection).unwrap_or_default();
        drop(inner);
        let _ = self.tx.send(LiveEvent {
            name: "detection",
            seq,
            json,
        });
    }

    /// Full current state as (sequence, JSON) for the first SSE event.
    fn snapshot(&self) -> (u64, String) {
        let inner = self.inner.lock().expect("observer mutex");
        let value = serde_json::json!({
            "type": "snapshot",
            "run_id": &inner.run_id,
            "t0_ms": inner.t0_ms.unwrap_or(0),
            "window_ms": WINDOW_MS,
            "bands": &inner.bands,
            "ground_truth": { "incidents": &inner.incidents },
            "oracle": &inner.oracle,
            "samples": inner.samples.iter().collect::<Vec<_>>(),
            "detections": &inner.detections,
        });
        (inner.seq, value.to_string())
    }

    fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.tx.subscribe()
    }

    /// The current state as a standalone JSON document (ADR-018 export).
    pub(crate) fn snapshot_json(&self) -> String {
        self.snapshot().1
    }

    /// The frozen state as one self-contained, offline-openable HTML document:
    /// CSS and JS are inlined and the snapshot is injected so the page renders
    /// without an HTTP server or SSE connection (ADR-018).
    pub(crate) fn export_html(&self) -> String {
        // Escape `<` so a string in the snapshot cannot close the script tag.
        let snapshot = self.snapshot_json().replace('<', "\\u003c");
        include_str!("index.html")
            .replace(
                r#"<link rel="stylesheet" href="/style.css">"#,
                &format!("<style>\n{}\n</style>", include_str!("style.css")),
            )
            .replace(
                r#"<script src="/app.js"></script>"#,
                &format!(
                    "<script>window.__OBSERVER_SNAPSHOT__ = {snapshot};</script>\n<script>\n{}\n</script>",
                    include_str!("app.js")
                ),
            )
    }

    /// Writes the self-contained document to `path` (ADR-018).
    pub(crate) fn dump_html(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.export_html())
    }
}

/// Builds the live sink passed to the subscription loop.
pub(crate) fn sink(handle: Handle) -> crate::uprotocol::Sink {
    Some(Arc::new(move |m: &Message| handle.record_message(m))
        as Arc<dyn Fn(&Message) + Send + Sync>)
}

/// Serves the static frontend and the SSE stream until shutdown.
pub(crate) async fn serve(addr: String, handle: Handle) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("observer listening on http://{addr}");
    serve_on(listener, handle).await
}

/// Serves on an already-bound listener (also used by the integration test).
async fn serve_on(listener: tokio::net::TcpListener, handle: Handle) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/health", get(|| async { "ok" }))
        .route("/events", get(events))
        .route("/snapshot.json", get(snapshot_json))
        .route("/export.html", get(export_html))
        .with_state(handle);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("index.html"),
    )
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("app.js"),
    )
}

async fn style_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("style.css"),
    )
}

async fn snapshot_json(State(handle): State<Handle>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        handle.snapshot_json(),
    )
}

async fn export_html(State(handle): State<Handle>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        handle.export_html(),
    )
}

async fn events(
    State(handle): State<Handle>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    // Subscribe before snapshotting so no delta between the two is lost;
    // deltas already contained in the snapshot are skipped by sequence number.
    let rx = handle.subscribe();
    let (seq, snapshot) = handle.snapshot();
    let first = futures_util::stream::once(async move {
        Ok::<Event, Infallible>(Event::default().event("snapshot").data(snapshot))
    });
    let deltas = futures_util::stream::unfold((rx, seq), |(mut rx, mut last)| async move {
        loop {
            match rx.recv().await {
                Ok(ev) if ev.seq <= last => continue,
                Ok(ev) => {
                    last = ev.seq;
                    let event = Event::default().event(ev.name).data(ev.json);
                    return Some((Ok::<Event, Infallible>(event), (rx, last)));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(first.chain(deltas)).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observer() -> Handle {
        Observer::new(Some("run-1".into()), None, Vec::new(), None)
    }

    fn sample(handle: &Handle, t: u64) {
        handle.record_message(&Message::Battery(BatteryEvent {
            timestamp_ms: t,
            temp_avg: Some(30.0),
            ..Default::default()
        }));
    }

    #[test]
    fn rebases_timestamps_to_the_first_sample() {
        let h = observer();
        sample(&h, 1_000);
        sample(&h, 1_100);
        let (_, snap) = h.snapshot();
        assert!(snap.contains("\"t0_ms\":1000"));
        assert!(snap.contains("\"timestamp_ms\":100"));
    }

    #[test]
    fn ring_buffer_keeps_only_the_window() {
        let h = observer();
        for t in (0..=25_000).step_by(100) {
            sample(&h, t);
        }
        let inner = h.inner.lock().unwrap();
        assert!(inner.samples.front().unwrap().timestamp_ms >= 25_000 - WINDOW_MS);
        assert!(inner.samples.len() <= (WINDOW_MS / 100 + 1) as usize);
    }

    #[test]
    fn detection_uses_the_evidence_timestamp() {
        let h = observer();
        sample(&h, 5_000);
        h.record_message(&Message::Fault(FaultEvent {
            fault_id: "BatteryTempGenerationGap".into(),
            detection_class: "STREAM_GENERATION_GAP".into(),
            level: "VIOLATION".into(),
            stage: Stage::Failed,
            baseline: false,
            evidence: FaultEvidence {
                timestamp_ms: Some(5_300),
                interval_ms: Some(300),
                ..Default::default()
            },
        }));
        let inner = h.inner.lock().unwrap();
        assert_eq!(inner.detections[0].at_ms, 300);
        assert_eq!(inner.detections[0].evidence.interval_ms, Some(300));
    }

    #[test]
    fn baseline_events_are_not_recorded() {
        let h = observer();
        h.record_message(&Message::Fault(FaultEvent {
            fault_id: "F".into(),
            detection_class: "STREAM_STALE".into(),
            level: "VIOLATION".into(),
            stage: Stage::Passed,
            baseline: true,
            evidence: FaultEvidence::default(),
        }));
        assert!(h.inner.lock().unwrap().detections.is_empty());
    }

    #[test]
    fn bands_come_from_the_authoritative_model() {
        let cfg = battery_guardian::GuardianConfig::from_yaml_str(include_str!(
            "../../../../config/battery_guardian/guardian_model.yaml"
        ))
        .unwrap();
        let bands = Bands::from_config(&cfg);
        assert_eq!(bands.temp_abs_max_c, 70.0);
        assert_eq!(bands.temp_warning_c, 60.0);
        assert_eq!(bands.temp_abs_min_c, -30.0);
    }

    #[test]
    fn export_html_inlines_assets_and_injects_snapshot() {
        let h = observer();
        sample(&h, 0);
        let html = h.export_html();
        assert!(html.contains("window.__OBSERVER_SNAPSHOT__ = {"), "{html}");
        assert!(html.contains("\"type\":\"snapshot\""), "{html}");
        assert!(!html.contains("src=\"/app.js\""), "{html}");
        assert!(!html.contains("href=\"/style.css\""), "{html}");
        assert!(html.contains("<style>"), "{html}");
    }

    #[test]
    fn snapshot_json_is_machine_readable() {
        let h = observer();
        sample(&h, 0);
        let json: serde_json::Value = serde_json::from_str(&h.snapshot_json()).unwrap();
        assert_eq!(json["type"], "snapshot");
        assert!(json["samples"].as_array().is_some());
    }

    /// The browser-facing contract: the first SSE frame is a `snapshot` that
    /// carries the model bands and the buffered samples.
    #[tokio::test]
    async fn serves_a_snapshot_first_over_sse() {
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = observer();
        handle.record_message(&Message::Battery(BatteryEvent {
            timestamp_ms: 100,
            temp_avg: Some(31.0),
            ..Default::default()
        }));
        let server = handle.clone();
        tokio::spawn(async move {
            let _ = serve_on(listener, server).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut text = String::new();
        let mut buf = [0u8; 1024];
        while !text.contains("\"bands\"") {
            match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => text.push_str(&String::from_utf8_lossy(&buf[..n])),
                _ => break,
            }
        }
        assert!(text.contains("event: snapshot"), "{text}");
        assert!(text.contains("\"bands\""), "{text}");
        assert!(text.contains("\"temp_avg\":31.0"), "{text}");
    }
}
