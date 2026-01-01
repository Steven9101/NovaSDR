use anyhow::Context;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};

const DEFAULT_BANDS_RAW: &str = include_str!("../resources/default_bands.json");

#[derive(Debug, Clone)]
pub struct OverlayPaths {
    pub dir: PathBuf,
    pub markers: PathBuf,
    pub bands: PathBuf,
}

pub fn overlay_paths_for_config(config_path: &Path) -> OverlayPaths {
    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let dir = config_dir.join("overlays");
    OverlayPaths {
        markers: dir.join("markers.json"),
        bands: dir.join("bands.json"),
        dir,
    }
}

pub fn ensure_default_overlays(config_path: &Path) -> anyhow::Result<OverlayPaths> {
    let paths = overlay_paths_for_config(config_path);

    std::fs::create_dir_all(&paths.dir)
        .with_context(|| format!("create overlays dir: {}", paths.dir.display()))?;

    write_json_if_missing(&paths.markers, &default_markers_value())
        .context("ensure overlays markers.json")?;
    write_json_if_missing(
        &paths.bands,
        &default_bands_value().context("load default bands")?,
    )
    .context("ensure overlays bands.json")?;

    Ok(paths)
}

pub fn default_markers_value() -> serde_json::Value {
    json!({ "markers": [] })
}

pub fn default_bands_value() -> anyhow::Result<serde_json::Value> {
    let v = serde_json::from_str::<serde_json::Value>(DEFAULT_BANDS_RAW)
        .context("parse default bands json")?;
    let _ = v
        .get("bands")
        .and_then(|b| b.as_array())
        .ok_or_else(|| anyhow::anyhow!("default bands json: expected {{\"bands\": [...]}}"))?;
    Ok(v)
}

fn write_json_if_missing(path: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
    use std::io::ErrorKind;

    let mut content = serde_json::to_string_pretty(value).context("serialize json")?;
    content.push('\n');

    let mut f = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => {
            return Err(e).with_context(|| format!("create file: {}", path.display()));
        }
    };

    f.write_all(content.as_bytes())
        .with_context(|| format!("write file: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ensure_default_overlays_creates_directory_and_files() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("novasdr_overlays_test_{ts}"));
        std::fs::create_dir_all(&root).unwrap();

        let config_path = root.join("config.json");
        std::fs::write(&config_path, "{}\n").unwrap();

        let paths = ensure_default_overlays(&config_path).unwrap();
        assert!(paths.dir.ends_with("overlays"));
        assert!(paths.markers.exists(), "markers.json should exist");
        assert!(paths.bands.exists(), "bands.json should exist");

        let markers: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.markers).unwrap()).unwrap();
        assert!(markers.get("markers").and_then(|v| v.as_array()).is_some());

        let bands: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.bands).unwrap()).unwrap();
        let bands_arr = bands.get("bands").and_then(|v| v.as_array()).unwrap();
        assert!(!bands_arr.is_empty(), "default bands should not be empty");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
