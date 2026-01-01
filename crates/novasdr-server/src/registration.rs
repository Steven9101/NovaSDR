use crate::{shutdown, state::AppState};
use anyhow::Context;
use reqwest::header::{HeaderMap, HeaderValue, HOST, USER_AGENT};
use serde::Serialize;
use std::{sync::Arc, time::Duration};

const UPDATE_INTERVAL: Duration = Duration::from_secs(60);
const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Serialize)]
struct SdrListUpdate<'a> {
    id: &'a str,
    name: &'a str,
    antenna: &'a str,
    bandwidth: i64,
    users: usize,
    center_frequency: i64,
    grid_locator: &'a str,
    hostname: &'a str,
    max_users: usize,
    port: u16,
}

pub fn spawn(state: Arc<AppState>) {
    if !state.cfg.websdr.register_online {
        tracing::info!("SDR list registration disabled (set websdr.register_online=true)");
        return;
    }

    let url = state.cfg.websdr.register_url.clone();
    tracing::info!(%url, "SDR list registration enabled");

    tokio::spawn(async move {
        let id = rand::random::<u32>().to_string();
        let client = match build_client(&url) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = ?e, "SDR list registration client init failed");
                return;
            }
        };

        let mut attempt: u32 = 0;
        while !shutdown::is_shutdown_requested() {
            let payload = build_payload(&state, &id);
            match send_update(&client, &url, payload).await {
                Ok(()) => {
                    attempt = 0;
                    tokio::time::sleep(UPDATE_INTERVAL).await;
                }
                Err(e) => {
                    attempt = attempt.saturating_add(1);
                    let backoff = compute_backoff(attempt);
                    tracing::warn!(
                        error = ?e,
                        attempt,
                        backoff_secs = backoff.as_secs(),
                        "SDR list registration failed"
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    });
}

fn build_payload<'a>(state: &'a AppState, id: &'a str) -> SdrListUpdate<'a> {
    let cfg = &state.cfg;
    let receiver = state.active_receiver_state();
    let rt = receiver.rt.as_ref();
    let input = &receiver.receiver.input;

    let mut bandwidth = input.sps;
    if rt.is_real {
        bandwidth /= 2;
    }

    let mut center_frequency = input.frequency;
    if center_frequency == 0 {
        center_frequency = bandwidth / 2;
    }

    SdrListUpdate {
        id,
        name: cfg.websdr.name.as_str(),
        antenna: cfg.websdr.antenna.as_str(),
        bandwidth,
        users: state.event_clients.len(),
        center_frequency,
        grid_locator: cfg.websdr.grid_locator.as_str(),
        hostname: cfg.websdr.hostname.as_str(),
        max_users: cfg.limits.audio,
        port: cfg.server.port,
    }
}

fn build_client(url: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("NovaSDR/registration (+https://github.com/Steven9101/NovaSDR)"),
    );

    if let Ok(parsed) = reqwest::Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            if host == "sdr-list.xyz" {
                headers.insert(HOST, HeaderValue::from_static("sdr-list.xyz"));
            }
        }
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(10))
        .build()
        .context("build reqwest client")
}

async fn send_update(
    client: &reqwest::Client,
    url: &str,
    payload: SdrListUpdate<'_>,
) -> anyhow::Result<()> {
    let res = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .context("POST update_websdr")?;

    let status = res.status();
    if !status.is_success() {
        let body = res
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));
        anyhow::bail!("HTTP {status}: {body}");
    }
    Ok(())
}

fn compute_backoff(attempt: u32) -> Duration {
    let shift = attempt.min(16);
    let mul = 1u64 << shift;
    let secs = BACKOFF_BASE.as_secs().saturating_mul(mul);
    Duration::from_secs(secs.min(BACKOFF_MAX.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_monotonic_and_capped() {
        let mut last = Duration::from_secs(0);
        for attempt in 1..64 {
            let d = compute_backoff(attempt);
            assert!(d >= last, "attempt {attempt}: {d:?} < {last:?}");
            assert!(
                d <= BACKOFF_MAX,
                "attempt {attempt}: {d:?} > {BACKOFF_MAX:?}"
            );
            last = d;
        }
    }
}
