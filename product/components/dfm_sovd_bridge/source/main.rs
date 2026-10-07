//! `dfm_sovd_bridge`: OpenSOVD server exposing DFM faults.
//!
//! Environment:
//! - `SOVD_URL`               base URL (default `http://127.0.0.1:7690/sovd`)
//! - `DFM_SOVD_PATH`          DFM entity path / SOVD component id (default `battery_guardian`)
//! - `DFM_SOVD_NAME`          component display name (default `Battery Guardian`)
//! - `DFM_QUERY_TIMEOUT_MS`   per-query IPC timeout (default 1000)
//! - `DFM_STARTUP_WAIT_S`     how long to wait for the DFM at startup (default 10)

use std::time::{Duration, Instant};

use anyhow::Context;
use dfm_lib::Iceoryx2DfmQuery;
use dfm_sovd_bridge::{DfmClient, build_component};
use opensovd_server::{Server, Topology};
use tokio::net::TcpListener;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn env_u64(key: &str, default: u64) -> anyhow::Result<u64> {
    match std::env::var(key) {
        Ok(v) => v.parse().with_context(|| format!("invalid {key}={v:?}")),
        Err(_) => Ok(default),
    }
}

/// Fetch the fault codes of `path`, retrying until the DFM answers or `wait` elapses.
async fn discover_fault_codes(dfm: &DfmClient, path: &str, wait: Duration) -> Vec<String> {
    let deadline = Instant::now() + wait;
    loop {
        match dfm.all_faults(path).await {
            Ok(faults) => return faults.into_iter().map(|f| f.code).collect(),
            Err(e) if Instant::now() < deadline => {
                tracing::debug!(error = %e, "DFM not ready yet, retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "DFM did not answer; serving without per-fault resources");
                return Vec::new();
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dfm_sovd_bridge=info,info".into()),
        )
        .init();

    let url = env_or("SOVD_URL", "http://127.0.0.1:7690/sovd");
    let path = env_or("DFM_SOVD_PATH", "battery_guardian");
    let name = env_or("DFM_SOVD_NAME", "Battery Guardian");
    let timeout = Duration::from_millis(env_u64("DFM_QUERY_TIMEOUT_MS", 1000)?);
    let startup_wait = Duration::from_secs(env_u64("DFM_STARTUP_WAIT_S", 10)?);

    let uri: http::Uri = url
        .parse()
        .with_context(|| format!("invalid SOVD_URL {url:?}"))?;
    let authority = uri
        .authority()
        .context("SOVD_URL must include host:port")?
        .to_string();

    let dfm = DfmClient::spawn(move || Iceoryx2DfmQuery::with_timeout(timeout))?;
    let codes = discover_fault_codes(&dfm, &path, startup_wait).await;
    tracing::info!(path = %path, faults = codes.len(), "DFM fault catalog discovered");

    let topology = Topology::new();
    topology
        .write()
        .await
        .add_component(build_component(&dfm, &path, &name, &codes)?);

    let listener = TcpListener::bind(&authority)
        .await
        .with_context(|| format!("bind {authority}"))?;
    let server = Server::builder()
        .base_uri(uri)?
        .listener(listener)
        .topology(topology)
        .layer(opensovd_extra::trace::server_layer())
        .build()?;

    tracing::info!(url = %url, component = %path, "DFM SOVD bridge listening");
    server.serve().await?;
    Ok(())
}
