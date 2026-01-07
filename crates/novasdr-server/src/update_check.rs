use crate::state;
use anyhow::Context;
use serde_json::Value;
use std::cmp::Ordering;
use std::time::Duration;

pub fn spawn(state: std::sync::Arc<state::AppState>) {
    if !state.cfg.updates.check_on_startup {
        return;
    }

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(e) = check_once(&state).await {
            tracing::debug!(error = ?e, "update check failed");
        }
    });
}

async fn check_once(state: &state::AppState) -> anyhow::Result<()> {
    let repo = state.cfg.updates.github_repo.trim();
    if repo.is_empty() {
        return Ok(());
    }

    let current = crate::build_info::version();
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = reqwest::Client::new()
        .get(url)
        .header(reqwest::header::USER_AGENT, "NovaSDR update check")
        .send()
        .await
        .context("GET releases/latest")?;

    let status = resp.status();
    let body = resp.text().await.context("read response body")?;
    if !status.is_success() {
        tracing::debug!(status = %status, body_len = body.len(), "update check http error");
        return Ok(());
    }

    let v: Value = serde_json::from_str(&body).context("parse response json")?;
    let Some(tag) = v.get("tag_name").and_then(Value::as_str) else {
        return Ok(());
    };
    let latest = tag.trim().trim_start_matches('v');

    let Some(ordering) = compare_versions(current, latest) else {
        tracing::debug!(current, latest, "update check: unparseable version");
        return Ok(());
    };
    if ordering != Ordering::Less {
        return Ok(());
    }

    let release_url = format!("https://github.com/{repo}/releases/tag/{tag}");
    tracing::warn!(
        current,
        latest,
        url = %release_url,
        "new version available"
    );
    tracing::info!(
        target: "novasdr_notice",
        current,
        latest,
        url = %release_url,
        "update available"
    );
    Ok(())
}

fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let a = parse_version(a)?;
    let b = parse_version(b)?;
    Some(a.cmp(&b))
}

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.trim().split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch_raw = it.next()?;
    let patch_digits: String = patch_raw.chars().take_while(|c| c.is_ascii_digit()).collect();
    let patch = patch_digits.parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_versions_orders_semver_triplets() {
        assert_eq!(
            compare_versions("0.2.6", "0.2.6"),
            Some(Ordering::Equal)
        );
        assert_eq!(compare_versions("0.2.6", "0.2.7"), Some(Ordering::Less));
        assert_eq!(
            compare_versions("0.2.10", "0.2.9"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_versions("1.0.0", "0.9.9"), Some(Ordering::Greater));
    }

    #[test]
    fn compare_versions_accepts_suffix_after_patch() {
        assert_eq!(
            compare_versions("0.2.6", "0.2.7-beta1"),
            Some(Ordering::Less)
        );
    }
}
