use crate::cli;
use anyhow::Context;
#[cfg(feature = "soapysdr")]
use inquire::MultiSelect;
use inquire::{Confirm, Select, Text};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
#[cfg(feature = "soapysdr")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "soapysdr")]
use std::sync::Arc;
#[cfg(feature = "soapysdr")]
use std::time::Duration;

mod ui {
    use std::io::{self, Write};

    pub fn blank() {
        let mut stderr = io::stderr().lock();
        if stderr.write_all(b"\n").is_err() {
            return;
        }
        if stderr.flush().is_err() {}
    }

    pub fn line(message: &str) {
        let mut stderr = io::stderr().lock();
        if stderr.write_all(message.as_bytes()).is_err() {
            return;
        }
        if stderr.write_all(b"\n").is_err() {
            return;
        }
        if stderr.flush().is_err() {}
    }

    #[cfg(feature = "soapysdr")]
    pub fn overwrite_line(message: &str) {
        let mut stderr = io::stderr().lock();
        if stderr.write_all(b"\r").is_err() {
            return;
        }
        if stderr.write_all(message.as_bytes()).is_err() {
            return;
        }
        if stderr.flush().is_err() {}
    }

    #[cfg(feature = "soapysdr")]
    pub fn overwrite_done(message: &str) {
        let mut stderr = io::stderr().lock();
        if stderr.write_all(b"\r").is_err() {
            return;
        }
        if stderr.write_all(message.as_bytes()).is_err() {
            return;
        }
        if stderr.write_all(b"\n").is_err() {
            return;
        }
        if stderr.flush().is_err() {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Explicit,
    Prompted,
    FirstLaunchSoapy,
}

pub fn ask_to_run_setup(args: &cli::Args, config_was_explicit: bool) -> anyhow::Result<bool> {
    ui::blank();
    if config_was_explicit {
        ui::line("NovaSDR could not find the config files you specified:");
    } else {
        ui::line("NovaSDR could not find its default config files:");
    }
    ui::line(&format!("- config:    {}", args.config.display()));
    ui::line(&format!("- receivers: {}", args.receivers.display()));
    ui::blank();

    Confirm::new("Open the setup wizard now?")
        .with_default(true)
        .prompt()
        .context("prompt open setup")
}

pub fn ask_to_run_setup_for_invalid_config(
    args: &cli::Args,
    cause: &anyhow::Error,
) -> anyhow::Result<bool> {
    ui::blank();
    ui::line("NovaSDR found config files but could not load them:");
    ui::line(&format!("- config:    {}", args.config.display()));
    ui::line(&format!("- receivers: {}", args.receivers.display()));
    ui::line(&format!("- cause:     {cause}"));
    ui::blank();

    Confirm::new("Open the setup wizard to fix them now?")
        .with_default(true)
        .prompt()
        .context("prompt open setup")
}

pub async fn run(args: &cli::Args, mode: RunMode) -> anyhow::Result<()> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("setup requires an interactive terminal");
    }

    ui::blank();
    ui::line("NovaSDR setup");
    ui::line(&format!("Version: {}", env!("CARGO_PKG_VERSION")));
    ui::blank();

    let config_path = args.config.clone();
    let receivers_path = args.receivers.clone();

    ui::line(&format!("Config:    {}", config_path.display()));
    ui::line(&format!("Receivers: {}", receivers_path.display()));
    ui::blank();

    if mode != RunMode::FirstLaunchSoapy
        && !Confirm::new("Continue?")
            .with_default(true)
            .prompt()
            .context("prompt continue")?
    {
        return Ok(());
    }

    let mut config = load_or_create_config(&config_path)?;
    let mut receivers = load_or_create_receivers(&receivers_path)?;

    if mode == RunMode::FirstLaunchSoapy && !receivers_path.exists() {
        seed_receivers_from_soapysdr(&mut receivers)?;
    }

    configure_global(&mut config)?;
    configure_receivers(&mut receivers)?;
    configure_active_receiver(&mut config, &receivers)?;
    configure_extras(&config_path)?;

    write_json(&config_path, &config)?;
    write_json(&receivers_path, &receivers)?;

    ui::blank();
    ui::line("Wrote:");
    ui::line(&format!("- {}", config_path.display()));
    ui::line(&format!("- {}", receivers_path.display()));
    ui::blank();

    Ok(())
}

fn load_or_create_config(path: &Path) -> anyhow::Result<Value> {
    if path.exists() {
        return read_json(path).context("read config file");
    }

    let create = Confirm::new(&format!(
        "{} does not exist. Create a minimal config file?",
        path.display()
    ))
    .with_default(true)
    .prompt()
    .context("prompt create config file")?;

    if !create {
        anyhow::bail!("config file is required");
    }

    Ok(json!({
      "server": { "port": 9002, "host": "[::]", "html_root": "frontend/dist/", "otherusers": 1, "threads": 1 },
      "websdr": { "register_online": false, "name": "NovaSDR", "antenna": "", "grid_locator": "-", "hostname": "", "operator": "", "email": "", "callsign_lookup_url": "https://www.qrz.com/db/", "chat_enabled": true },
      "limits": { "audio": 1000, "waterfall": 1000, "events": 1000 }
    }))
}

fn load_or_create_receivers(path: &Path) -> anyhow::Result<Value> {
    if path.exists() {
        return read_json(path).context("read receivers file");
    }

    let create = Confirm::new(&format!(
        "{} does not exist. Create a minimal receivers file?",
        path.display()
    ))
    .with_default(true)
    .prompt()
    .context("prompt create receivers file")?;

    if !create {
        anyhow::bail!("receivers file is required");
    }

    Ok(json!({ "receivers": [] }))
}

fn seed_receivers_from_soapysdr(receivers: &mut Value) -> anyhow::Result<()> {
    if !cfg!(feature = "soapysdr") {
        let _ = receivers;
        return Ok(());
    }

    #[cfg(feature = "soapysdr")]
    {
        let discovered = discover_soapysdr_devices()?;
        if discovered.is_empty() {
            ui::line("No SoapySDR devices detected.");
            return Ok(());
        }

        ui::blank();
        ui::line("Detected SoapySDR devices");
        ui::line("------------------------");

        let selected = MultiSelect::new("Select devices to add as receivers", discovered)
            .prompt()
            .context("prompt device selection")?;
        if selected.is_empty() {
            return Ok(());
        }

        let receivers_arr = receivers
            .get_mut("receivers")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow::anyhow!("receivers.json: missing receivers[] array"))?;

        receivers_arr.clear();

        for (idx, dev) in selected.iter().enumerate() {
            let id = format!("rx{idx}");
            receivers_arr.push(json!({
              "id": id,
              "name": display_name_for_device(dev),
              "input": {
                "sps": 2048000,
                "frequency": 100900000,
                "signal": "iq",
                "fft_size": 131072,
                "brightness_offset": 0,
                "audio_sps": 12000,
                "waterfall_size": 1024,
                "waterfall_compression": "zstd",
                "audio_compression": "flac",
                "smeter_offset": 0,
                "accelerator": "none",
                "driver": { "kind": "soapysdr", "device": dev, "channel": 0, "format": "cs16" },
                "defaults": { "frequency": -1, "modulation": "USB" }
              }
            }));
        }

        Ok(())
    }

    #[cfg(not(feature = "soapysdr"))]
    {
        let _ = receivers;
        Ok(())
    }
}

#[cfg(feature = "soapysdr")]
fn discover_soapysdr_devices() -> anyhow::Result<Vec<String>> {
    let mut out = Vec::<String>::new();
    let mut seen = std::collections::HashSet::<String>::new();
    let devs = with_spinner("Scanning SoapySDR devices", || soapysdr::enumerate(""))
        .context("enumerate SoapySDR devices")?;
    for d in devs {
        let s = d.to_string();
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    Ok(out)
}

#[cfg(feature = "soapysdr")]
fn display_name_for_device(dev: &str) -> String {
    let d = dev.trim();
    if d.is_empty() {
        return "SoapySDR".to_string();
    }
    if let Some(pos) = d.find("driver=") {
        let tail = &d[pos + "driver=".len()..];
        let end = tail.find(',').unwrap_or(tail.len());
        let driver = tail[..end].trim();
        if !driver.is_empty() {
            return format!("SoapySDR ({driver})");
        }
    }
    "SoapySDR".to_string()
}

fn configure_global(cfg: &mut Value) -> anyhow::Result<()> {
    ui::blank();
    ui::line("Global settings");
    ui::line("----------------");

    let Some(server) = cfg.get_mut("server").and_then(Value::as_object_mut) else {
        anyhow::bail!("config.json: missing object server");
    };

    let port = prompt_u16(
        "Server port",
        server.get("port").and_then(Value::as_u64).unwrap_or(9002) as u16,
    )?;
    server.insert("port".to_string(), json!(port));

    let host = Text::new("Bind host")
        .with_default(server.get("host").and_then(Value::as_str).unwrap_or("[::]"))
        .prompt()
        .context("prompt host")?;
    server.insert("host".to_string(), json!(host));

    let html_root = Text::new("Static UI directory (server.html_root)")
        .with_default(
            server
                .get("html_root")
                .and_then(Value::as_str)
                .unwrap_or("frontend/dist/"),
        )
        .prompt()
        .context("prompt html_root")?;
    server.insert("html_root".to_string(), json!(html_root));

    let Some(websdr) = cfg.get_mut("websdr").and_then(Value::as_object_mut) else {
        anyhow::bail!("config.json: missing object websdr");
    };

    let name = Text::new("WebSDR name")
        .with_default(
            websdr
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("NovaSDR"),
        )
        .prompt()
        .context("prompt websdr.name")?;
    websdr.insert("name".to_string(), json!(name));

    let grid = Text::new("Grid locator (for UI)")
        .with_default(
            websdr
                .get("grid_locator")
                .and_then(Value::as_str)
                .unwrap_or("-"),
        )
        .prompt()
        .context("prompt websdr.grid_locator")?;
    websdr.insert("grid_locator".to_string(), json!(grid));

    Ok(())
}

fn configure_extras(config_path: &Path) -> anyhow::Result<()> {
    let overlay_paths = crate::overlays::overlay_paths_for_config(config_path);
    let markers_existed = overlay_paths.markers.exists();
    let bands_existed = overlay_paths.bands.exists();

    let paths = crate::overlays::ensure_default_overlays(config_path).context("init overlays")?;
    let markers_path = paths.markers;
    let bands_path = paths.bands;

    ui::blank();
    ui::line("Overlays (bands) and markers");
    ui::line("----------------------------");

    if !bands_existed {
        let use_defaults =
            Confirm::new("Initialize overlays/bands.json with the default band plan?")
                .with_default(true)
                .prompt()
                .context("prompt init bands defaults")?;
        if !use_defaults {
            write_json(&bands_path, &json!({ "bands": [] }))?;
        }

        let edit_now = Confirm::new("Edit band plan now?")
            .with_default(false)
            .prompt()
            .context("prompt edit bands now")?;
        if edit_now {
            edit_bands(&bands_path)?;
        }
    }

    if !markers_existed {
        let edit_now = Confirm::new("Add markers now?")
            .with_default(false)
            .prompt()
            .context("prompt add markers now")?;
        if edit_now {
            edit_markers(&markers_path)?;
        }
    }

    loop {
        let choice = Select::new(
            "What do you want to do?",
            vec![
                format!("Markers ({})", markers_path.display()),
                format!("Bands ({})", bands_path.display()),
                "Markers: clear (empty)".to_string(),
                "Bands: reset to default band plan".to_string(),
                "Bands: start empty".to_string(),
                "Done".to_string(),
            ],
        )
        .prompt()
        .context("prompt extras selection")?;

        if choice == "Done" {
            break;
        }
        if choice.starts_with("Markers") {
            edit_markers(&markers_path)?;
        } else if choice.starts_with("Bands") {
            edit_bands(&bands_path)?;
        } else if choice == "Markers: clear (empty)" {
            let ok = Confirm::new("Replace overlays/markers.json with an empty markers list?")
                .with_default(false)
                .prompt()
                .context("prompt clear markers")?;
            if ok {
                write_json(&markers_path, &crate::overlays::default_markers_value())?;
            }
        } else if choice == "Bands: reset to default band plan" {
            let ok = Confirm::new("Reset overlays/bands.json to the default band plan?")
                .with_default(false)
                .prompt()
                .context("prompt reset bands default")?;
            if ok {
                let v = crate::overlays::default_bands_value().context("load default bands")?;
                write_json(&bands_path, &v)?;
            }
        } else if choice == "Bands: start empty" {
            let ok = Confirm::new("Replace overlays/bands.json with an empty bands list?")
                .with_default(false)
                .prompt()
                .context("prompt empty bands")?;
            if ok {
                write_json(&bands_path, &json!({ "bands": [] }))?;
            }
        }
    }

    Ok(())
}

fn edit_markers(path: &Path) -> anyhow::Result<()> {
    let mut root = if path.exists() {
        read_json(path).context("read markers.json")?
    } else {
        crate::overlays::default_markers_value()
    };

    let arr = root
        .get_mut("markers")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("markers.json: expected {{\"markers\": [...]}}"))?;

    loop {
        let mut choices = Vec::<String>::new();
        for m in arr.iter() {
            let hz = m.get("frequency").and_then(Value::as_i64).unwrap_or(0);
            let name = m.get("name").and_then(Value::as_str).unwrap_or("");
            let mode = m.get("mode").and_then(Value::as_str).unwrap_or("");
            choices.push(format!("{hz} Hz  {name}  {mode}"));
        }
        choices.push("Add marker".to_string());
        choices.push("Delete marker".to_string());
        choices.push("Back".to_string());

        let selected = Select::new("Markers", choices)
            .prompt()
            .context("prompt markers menu")?;

        if selected == "Back" {
            break;
        }
        if selected == "Add marker" {
            let hz = prompt_frequency_hz("Marker frequency", 7_074_000)?;
            let name = Text::new("Marker name").with_default("Marker").prompt()?;
            let mode = Select::new(
                "Marker modulation (optional)",
                vec![
                    "(none)".to_string(),
                    "USB".to_string(),
                    "LSB".to_string(),
                    "CW".to_string(),
                    "AM".to_string(),
                    "SAM".to_string(),
                    "FM".to_string(),
                    "WBFM".to_string(),
                ],
            )
            .prompt()?;
            let mut marker = json!({ "frequency": hz, "name": name });
            if mode != "(none)" {
                let obj = marker.as_object_mut().ok_or_else(|| {
                    anyhow::anyhow!("internal error: marker json is not an object")
                })?;
                obj.insert("mode".to_string(), json!(mode));
            }
            arr.push(marker);
            continue;
        }
        if selected == "Delete marker" {
            if arr.is_empty() {
                continue;
            }
            let idx = Select::new(
                "Select marker to delete",
                arr.iter()
                    .map(|m| {
                        let hz = m.get("frequency").and_then(Value::as_i64).unwrap_or(0);
                        let name = m.get("name").and_then(Value::as_str).unwrap_or("");
                        format!("{hz} Hz  {name}")
                    })
                    .collect::<Vec<_>>(),
            )
            .prompt()?;
            if let Some(pos) = arr.iter().position(|m| {
                let hz = m.get("frequency").and_then(Value::as_i64).unwrap_or(0);
                idx.starts_with(&format!("{hz} Hz"))
            }) {
                arr.remove(pos);
            }
            continue;
        }
    }

    write_json(path, &root)
}

fn edit_bands(path: &Path) -> anyhow::Result<()> {
    let mut root = if path.exists() {
        read_json(path).context("read bands.json")?
    } else {
        crate::overlays::default_bands_value().context("load default bands")?
    };

    let arr = root
        .get_mut("bands")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("bands.json: expected {{\"bands\": [...]}}"))?;

    loop {
        let mut choices = Vec::<String>::new();
        for b in arr.iter() {
            let name = b.get("name").and_then(Value::as_str).unwrap_or("");
            let start = b.get("startHz").and_then(Value::as_i64).unwrap_or(0);
            let end = b.get("endHz").and_then(Value::as_i64).unwrap_or(0);
            choices.push(format!("{name}  {start}..{end}"));
        }
        choices.push("Add band".to_string());
        choices.push("Delete band".to_string());
        choices.push("Back".to_string());

        let selected = Select::new("Bands", choices)
            .prompt()
            .context("prompt bands menu")?;

        if selected == "Back" {
            break;
        }
        if selected == "Add band" {
            let name = Text::new("Band name").with_default("Band").prompt()?;
            let start = prompt_frequency_hz("Start frequency", 144_000_000)?;
            let end = prompt_frequency_hz("End frequency", 146_000_000)?;
            let color = Text::new("Color (CSS, blank to skip)")
                .with_default("")
                .prompt()?;
            let mut band = json!({ "name": name, "startHz": start, "endHz": end });
            if !color.trim().is_empty() {
                let obj = band
                    .as_object_mut()
                    .ok_or_else(|| anyhow::anyhow!("internal error: band json is not an object"))?;
                obj.insert("color".to_string(), json!(color));
            }
            arr.push(band);
            continue;
        }
        if selected == "Delete band" {
            if arr.is_empty() {
                continue;
            }
            let idx = Select::new(
                "Select band to delete",
                arr.iter()
                    .map(|b| {
                        let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                        let start = b.get("startHz").and_then(Value::as_i64).unwrap_or(0);
                        let end = b.get("endHz").and_then(Value::as_i64).unwrap_or(0);
                        format!("{name}  {start}..{end}")
                    })
                    .collect::<Vec<_>>(),
            )
            .prompt()?;
            if let Some(pos) = arr.iter().position(|b| {
                let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                idx.starts_with(name)
            }) {
                arr.remove(pos);
            }
            continue;
        }
    }

    write_json(path, &root)
}

fn configure_active_receiver(config: &mut Value, receivers: &Value) -> anyhow::Result<()> {
    let ids = receivers
        .get("receivers")
        .and_then(Value::as_array)
        .map_or(&[][..], |v| v.as_slice())
        .iter()
        .filter_map(|r| r.get("id").and_then(Value::as_str))
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    if ids.is_empty() {
        return Ok(());
    }

    let current = config
        .get("active_receiver_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let default = current
        .and_then(|c| ids.iter().position(|id| id == &c))
        .unwrap_or(0);

    let selected = Select::new("Default receiver (active_receiver_id)", ids)
        .with_starting_cursor(default)
        .prompt()
        .context("prompt active receiver")?;
    config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("config.json must be an object"))?
        .insert("active_receiver_id".to_string(), json!(selected));
    Ok(())
}

fn configure_receivers(root: &mut Value) -> anyhow::Result<()> {
    ui::blank();
    ui::line("Receivers");
    ui::line("---------");

    let receivers = root
        .get_mut("receivers")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("receivers.json: missing receivers[] array"))?;

    loop {
        let mut choices: Vec<String> = receivers
            .iter()
            .filter_map(|r| r.get("id").and_then(Value::as_str).map(|s| s.to_string()))
            .collect();
        choices.push("Add receiver".to_string());
        if !receivers.is_empty() {
            choices.push("Remove receiver".to_string());
            choices.push("Done".to_string());
        }

        let selected = Select::new("Select receiver to edit", choices)
            .prompt()
            .context("prompt receiver selection")?;

        if selected == "Done" {
            break;
        }

        if selected == "Add receiver" {
            let mut used = HashSet::<String>::new();
            for r in receivers.iter() {
                if let Some(id) = r.get("id").and_then(Value::as_str) {
                    used.insert(id.to_string());
                }
            }
            let default_id = if receivers.is_empty() {
                "rx0".to_string()
            } else {
                let mut i = 0usize;
                loop {
                    let candidate = format!("rx{i}");
                    if !used.contains(&candidate) {
                        break candidate;
                    }
                    i += 1;
                }
            };

            let id = Text::new("Receiver id (unique)")
                .with_default(&default_id)
                .prompt()
                .context("prompt receiver id")?;
            let id = id.trim().to_string();
            if id.is_empty() {
                ui::line("Receiver id must not be empty.");
                continue;
            }
            if receivers
                .iter()
                .any(|r| r.get("id").and_then(Value::as_str) == Some(id.as_str()))
            {
                ui::line("Receiver id already exists.");
                continue;
            }

            let mut entry = json!({
              "id": id,
              "name": "",
              "input": {
                "sps": 2048000,
                "frequency": 100900000,
                "signal": "iq",
                "fft_size": 131072,
                "brightness_offset": 0,
                "audio_sps": 12000,
                "waterfall_size": 1024,
                "waterfall_compression": "zstd",
                "audio_compression": "flac",
                "smeter_offset": 0,
                "accelerator": "none",
                "driver": { "kind": "stdin", "format": "u8" },
                "defaults": { "frequency": -1, "modulation": "USB", "squelch_enabled": false }
              }
            });
            edit_receiver(&mut entry)?;
            receivers.push(entry);
            continue;
        }

        if selected == "Remove receiver" {
            let ids: Vec<String> = receivers
                .iter()
                .filter_map(|r| r.get("id").and_then(Value::as_str).map(|s| s.to_string()))
                .collect();
            if ids.is_empty() {
                ui::line("No receivers to remove.");
                continue;
            }

            let victim = Select::new("Select receiver to remove", ids)
                .prompt()
                .context("prompt receiver removal selection")?;

            let confirm_msg = format!("Remove receiver {victim}?");
            let confirm = Confirm::new(&confirm_msg)
                .with_default(false)
                .prompt()
                .context("prompt receiver removal confirm")?;
            if !confirm {
                continue;
            }

            receivers.retain(|r| r.get("id").and_then(Value::as_str) != Some(victim.as_str()));
            ui::line(&format!("Removed receiver {victim}."));
            continue;
        }

        let Some(idx) = receivers.iter().position(|r| {
            r.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == selected.as_str())
        }) else {
            continue;
        };

        edit_receiver(&mut receivers[idx])?;
    }

    Ok(())
}

fn edit_receiver(receiver: &mut Value) -> anyhow::Result<()> {
    let receiver_id = receiver
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("receiver")
        .to_string();

    ui::blank();
    ui::line(&format!("Receiver: {receiver_id}"));
    ui::line("----------------");

    let name = Text::new("Display name")
        .with_default(receiver.get("name").and_then(Value::as_str).unwrap_or(""))
        .prompt()
        .context("prompt receiver name")?;
    receiver
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("receiver must be an object"))?
        .insert("name".to_string(), json!(name));

    let Some(input) = receiver.get_mut("input").and_then(Value::as_object_mut) else {
        anyhow::bail!("receiver.input must be an object");
    };

    let frequency_hz = prompt_center_frequency_hz(
        "Center frequency",
        input
            .get("frequency")
            .and_then(Value::as_i64)
            .unwrap_or(100_900_000),
    )?;
    input.insert("frequency".to_string(), json!(frequency_hz));

    let sps = prompt_sample_rate_sps(
        "Sample rate",
        input
            .get("sps")
            .and_then(Value::as_i64)
            .unwrap_or(2_048_000),
    )?;
    input.insert("sps".to_string(), json!(sps));

    let signal = Select::new("Signal type", vec!["iq".to_string(), "real".to_string()])
        .prompt()
        .context("prompt signal type")?;
    input.insert("signal".to_string(), json!(signal));

    let fft_size = prompt_usize(
        "FFT size (power of two)",
        input
            .get("fft_size")
            .and_then(Value::as_u64)
            .unwrap_or(131_072) as usize,
    )?;
    input.insert("fft_size".to_string(), json!(fft_size));

    let audio_sps = prompt_audio_rate_hz(
        "Audio sample rate target (audio_sps)",
        input
            .get("audio_sps")
            .and_then(Value::as_i64)
            .unwrap_or(12_000),
    )?;
    input.insert("audio_sps".to_string(), json!(audio_sps));

    let waterfall_size = prompt_usize(
        "Waterfall width (waterfall_size)",
        input
            .get("waterfall_size")
            .and_then(Value::as_u64)
            .unwrap_or(1024) as usize,
    )?;
    input.insert("waterfall_size".to_string(), json!(waterfall_size));

    {
        let current = input
            .get("accelerator")
            .and_then(Value::as_str)
            .unwrap_or("none");

        let mut labels = Vec::<String>::new();
        labels.push("none".to_string());

        if cfg!(feature = "clfft") {
            labels.push("clfft".to_string());
        } else {
            labels.push("clfft (requires --features clfft)".to_string());
        }

        if cfg!(feature = "vkfft") {
            labels.push("vkfft".to_string());
        } else {
            labels.push("vkfft (requires --features vkfft)".to_string());
        }

        let default_idx = labels.iter().position(|s| s == current).unwrap_or(0);

        let selected = Select::new("DSP accelerator (accelerator)", labels)
            .with_starting_cursor(default_idx)
            .prompt()
            .context("prompt accelerator")?;

        let value = if selected.starts_with("clfft") {
            if !cfg!(feature = "clfft") {
                ui::line(
                    "Note: this binary was built without the \"clfft\" feature. Selecting clfft will require rebuilding novasdr-server with --features clfft.",
                );
            }
            "clfft"
        } else if selected.starts_with("vkfft") {
            if !cfg!(feature = "vkfft") {
                ui::line(
                    "Note: this binary was built without the \"vkfft\" feature. Selecting vkfft will require rebuilding novasdr-server with --features vkfft.",
                );
            }
            "vkfft"
        } else {
            "none"
        };
        input.insert("accelerator".to_string(), json!(value));
    }

    let defaults = input
        .entry("defaults")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("receiver.input.defaults must be an object"))?;

    let default_frequency = prompt_frequency_hz_or_center(
        "Default tuning frequency",
        defaults
            .get("frequency")
            .and_then(Value::as_i64)
            .unwrap_or(-1),
    )?;
    defaults.insert("frequency".to_string(), json!(default_frequency));

    let modulation = Select::new(
        "Default modulation",
        vec![
            "USB".to_string(),
            "LSB".to_string(),
            "CW".to_string(),
            "AM".to_string(),
            "SAM".to_string(),
            "FM".to_string(),
            "WBFM".to_string(),
        ],
    )
    .prompt()
    .context("prompt default modulation")?;
    defaults.insert("modulation".to_string(), json!(modulation));

    let modulation_upper = defaults
        .get("modulation")
        .and_then(Value::as_str)
        .unwrap_or("USB")
        .to_ascii_uppercase();
    if modulation_upper == "USB" || modulation_upper == "LSB" {
        let current_lowcut = defaults
            .get("ssb_lowcut_hz")
            .and_then(Value::as_i64)
            .unwrap_or(300);
        let current_highcut = defaults
            .get("ssb_highcut_hz")
            .and_then(Value::as_i64)
            .unwrap_or(3000);

        loop {
            let lowcut = prompt_passband_hz("SSB low-cut (ssb_lowcut_hz)", current_lowcut)?;
            let highcut = prompt_passband_hz("SSB high-cut (ssb_highcut_hz)", current_highcut)?;
            if highcut <= lowcut {
                ui::line("Invalid SSB passband: high-cut must be greater than low-cut.");
                continue;
            }
            defaults.insert("ssb_lowcut_hz".to_string(), json!(lowcut));
            defaults.insert("ssb_highcut_hz".to_string(), json!(highcut));
            break;
        }
    }

    let Some(driver) = input.get_mut("driver").and_then(Value::as_object_mut) else {
        anyhow::bail!("receiver.input.driver must be an object");
    };

    let kind = Select::new(
        "Input driver",
        vec!["stdin".to_string(), "soapysdr".to_string()],
    )
    .prompt()
    .context("prompt driver kind")?;
    driver.insert("kind".to_string(), json!(kind.clone()));

    match kind.as_str() {
        "stdin" => {
            driver.remove("device");
            driver.remove("channel");
            driver.remove("antenna");
            driver.remove("agc");
            driver.remove("gain");
            driver.remove("gains");
            driver.remove("settings");

            let format = Select::new(
                "Sample format",
                vec![
                    "u8".to_string(),
                    "s8".to_string(),
                    "u16".to_string(),
                    "s16".to_string(),
                    "cs16".to_string(),
                    "cf32".to_string(),
                    "f32".to_string(),
                    "f64".to_string(),
                ],
            )
            .prompt()
            .context("prompt stdin format")?;
            driver.insert("format".to_string(), json!(format));
        }
        "soapysdr" => {
            if !cfg!(feature = "soapysdr") {
                ui::line(
                    "Note: this binary was built without the \"soapysdr\" feature. Device scanning is unavailable.",
                );
            }

            let device =
                prompt_soapysdr_device(driver.get("device").and_then(Value::as_str).unwrap_or(""))?;
            driver.insert("device".to_string(), json!(device));

            let channel = prompt_usize(
                "RX channel",
                driver.get("channel").and_then(Value::as_u64).unwrap_or(0) as usize,
            )?;
            driver.insert("channel".to_string(), json!(channel));

            let antenna = Text::new("Antenna (blank to skip)")
                .with_default(driver.get("antenna").and_then(Value::as_str).unwrap_or(""))
                .prompt()
                .context("prompt antenna")?;
            if antenna.trim().is_empty() {
                driver.remove("antenna");
            } else {
                driver.insert("antenna".to_string(), json!(antenna));
            }

            let format = Select::new(
                "SoapySDR format",
                vec!["cs16".to_string(), "cf32".to_string()],
            )
            .prompt()
            .context("prompt soapysdr format")?;
            driver.insert("format".to_string(), json!(format));

            let agc_mode = Select::new(
                "AGC mode",
                vec!["Default".to_string(), "On".to_string(), "Off".to_string()],
            )
            .prompt()
            .context("prompt agc mode")?;
            match agc_mode.as_str() {
                "On" => {
                    driver.insert("agc".to_string(), json!(true));
                }
                "Off" => {
                    driver.insert("agc".to_string(), json!(false));
                }
                _ => {
                    driver.remove("agc");
                }
            }

            let gain_default = driver
                .get("gain")
                .and_then(Value::as_f64)
                .map(|v| v.to_string())
                .unwrap_or_default();
            let gain_raw = Text::new("Overall gain dB (blank to keep device default)")
                .with_default(gain_default.as_str())
                .prompt()
                .context("prompt gain")?;
            let gain_raw = gain_raw.trim();
            if gain_raw.is_empty() {
                driver.remove("gain");
            } else {
                let gain: f64 = gain_raw.parse().context("parse gain as number")?;
                driver.insert("gain".to_string(), json!(gain));
            }

            configure_gain_elements(driver)?;
            configure_device_settings(driver)?;
        }
        _ => {}
    }

    Ok(())
}

fn prompt_soapysdr_device(current: &str) -> anyhow::Result<String> {
    if !cfg!(feature = "soapysdr") {
        return Text::new("SoapySDR device string")
            .with_default(current)
            .prompt()
            .context("prompt soapysdr device");
    }

    #[cfg(feature = "soapysdr")]
    {
        let mut choices: Vec<String> = Vec::new();
        match with_spinner("Scanning SoapySDR devices", || soapysdr::enumerate("")) {
            Ok(devs) => {
                let mut seen = std::collections::HashSet::<String>::new();
                for d in devs {
                    let s = d.to_string();
                    if seen.insert(s.clone()) {
                        choices.push(s);
                    }
                }
            }
            Err(e) => {
                ui::line(&format!("Device enumeration failed: {e:?}"));
            }
        }
        choices.push("Enter manually...".to_string());

        let selected = Select::new("SoapySDR device", choices)
            .prompt()
            .context("prompt soapysdr device selection")?;
        if selected == "Enter manually..." {
            return Text::new("SoapySDR device string")
                .with_default(current)
                .prompt()
                .context("prompt soapysdr device");
        }
        Ok(selected)
    }

    #[cfg(not(feature = "soapysdr"))]
    Ok(current.to_string())
}

fn configure_gain_elements(driver: &mut serde_json::Map<String, Value>) -> anyhow::Result<()> {
    if !cfg!(feature = "soapysdr") {
        return Ok(());
    }

    let device = driver.get("device").and_then(Value::as_str).unwrap_or("");
    if device.trim().is_empty() {
        return Ok(());
    }

    let configure = Confirm::new("Configure per-stage gain elements? (LNA/VGA/etc)")
        .with_default(false)
        .prompt()
        .context("prompt configure gain elements")?;
    if !configure {
        driver.remove("gains");
        return Ok(());
    }

    #[cfg(feature = "soapysdr")]
    {
        let channel = driver.get("channel").and_then(Value::as_u64).unwrap_or(0) as usize;
        let device = soapysdr::Device::new(device)
            .context("open SoapySDR device for gain element enumeration")?;
        let gains = device
            .list_gains(soapysdr::Direction::Rx, channel)
            .context("list gain elements")?;

        let mut out = BTreeMap::<String, f64>::new();
        for name in gains {
            let raw = Text::new(&format!("Gain element {name} dB (blank to skip)"))
                .with_default("")
                .prompt()
                .with_context(|| format!("prompt gain element {name}"))?;
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let v: f64 = raw.parse().with_context(|| format!("parse {name} gain"))?;
            out.insert(name, v);
        }

        if out.is_empty() {
            driver.remove("gains");
        } else {
            driver.insert("gains".to_string(), serde_json::to_value(out)?);
        }
        Ok(())
    }

    #[cfg(not(feature = "soapysdr"))]
    Ok(())
}

fn configure_device_settings(driver: &mut serde_json::Map<String, Value>) -> anyhow::Result<()> {
    let configure = Confirm::new("Add SoapySDR device settings? (key=value)")
        .with_default(false)
        .prompt()
        .context("prompt configure device settings")?;
    if !configure {
        driver.remove("settings");
        return Ok(());
    }

    let mut out = BTreeMap::<String, String>::new();
    loop {
        let key = Text::new("Setting key (blank to finish)")
            .with_default("")
            .prompt()
            .context("prompt setting key")?;
        let key = key.trim().to_string();
        if key.is_empty() {
            break;
        }
        let value = Text::new("Setting value")
            .with_default("")
            .prompt()
            .context("prompt setting value")?;
        out.insert(key, value);
    }

    if out.is_empty() {
        driver.remove("settings");
    } else {
        driver.insert("settings".to_string(), serde_json::to_value(out)?);
    }
    Ok(())
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn write_json(path: &Path, v: &Value) -> anyhow::Result<()> {
    let raw = serde_json::to_string_pretty(v).context("serialize json")?;
    atomic_write(path, raw.as_bytes()).with_context(|| format!("write {}", path.display()))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create directory {}", parent.display()))?;

    let tmp = tmp_path_for(path);
    std::fs::write(&tmp, bytes).with_context(|| format!("write temp {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "config".to_string());
    name.push_str(".tmp");
    path.with_file_name(name)
}

fn prompt_u16(label: &str, default: u16) -> anyhow::Result<u16> {
    let raw = Text::new(label)
        .with_default(&default.to_string())
        .prompt()
        .with_context(|| format!("prompt {label}"))?;
    raw.trim()
        .parse::<u16>()
        .with_context(|| format!("parse {label}"))
}

fn prompt_usize(label: &str, default: usize) -> anyhow::Result<usize> {
    let raw = Text::new(label)
        .with_default(&default.to_string())
        .prompt()
        .with_context(|| format!("prompt {label}"))?;
    raw.trim()
        .parse::<usize>()
        .with_context(|| format!("parse {label}"))
}

fn prompt_frequency_hz(label: &str, default_hz: i64) -> anyhow::Result<i64> {
    let default_str = format_khz((default_hz as f64) / 1_000.0);
    loop {
        let raw = Text::new(&format!(
            "{label} (kHz default; examples: 100mhz, 100khz, 100hz)"
        ))
        .with_default(default_str.as_str())
        .prompt()
        .with_context(|| format!("prompt {label}"))?;
        match parse_frequency_to_hz(raw.as_str()) {
            Ok(hz) => return Ok(hz),
            Err(e) => ui::line(&format!("Invalid frequency: {e}")),
        }
    }
}

fn prompt_center_frequency_hz(label: &str, default_hz: i64) -> anyhow::Result<i64> {
    let default_str = format_khz((default_hz as f64) / 1_000.0);
    loop {
        let raw = Text::new(&format!(
            "{label} (kHz default; examples: 0, 100mhz, 100khz, 100hz)"
        ))
        .with_default(default_str.as_str())
        .prompt()
        .with_context(|| format!("prompt {label}"))?;
        match parse_frequency_to_hz_allow_zero(raw.as_str()) {
            Ok(hz) => return Ok(hz),
            Err(e) => ui::line(&format!("Invalid frequency: {e}")),
        }
    }
}

fn prompt_frequency_hz_or_center(label: &str, default_hz_or_center: i64) -> anyhow::Result<i64> {
    let default_str = if default_hz_or_center == -1 {
        "-1".to_string()
    } else {
        format_khz((default_hz_or_center as f64) / 1_000.0)
    };

    loop {
        let raw = Text::new(&format!(
            "{label} (-1 = center; kHz default; examples: -1, 7074, 7.074mhz)"
        ))
        .with_default(default_str.as_str())
        .prompt()
        .with_context(|| format!("prompt {label}"))?;
        let s = raw.trim();
        if s == "-1" {
            return Ok(-1);
        }
        match parse_frequency_to_hz(s) {
            Ok(hz) => return Ok(hz),
            Err(e) => ui::line(&format!("Invalid frequency: {e}")),
        }
    }
}

fn parse_frequency_to_hz(raw: &str) -> anyhow::Result<i64> {
    let s0 = raw.trim();
    anyhow::ensure!(!s0.is_empty(), "empty input");

    let s = s0
        .to_ascii_lowercase()
        .replace(['_', ' '], "")
        .replace(',', ".");

    let (number, multiplier): (&str, f64) = if let Some(n) = s.strip_suffix("mhz") {
        (n, 1_000_000.0)
    } else if let Some(n) = s.strip_suffix("khz") {
        (n, 1_000.0)
    } else if let Some(n) = s.strip_suffix("hz") {
        (n, 1.0)
    } else {
        (s.as_str(), 1_000.0)
    };

    let v: f64 = number.trim().parse().context("parse number")?;
    anyhow::ensure!(v.is_finite() && v > 0.0, "value must be > 0");
    let hz = (v * multiplier).round();
    anyhow::ensure!(hz >= 1.0, "value must be > 0");
    anyhow::ensure!(hz <= (i64::MAX as f64), "value too large");
    Ok(hz as i64)
}

fn parse_frequency_to_hz_allow_zero(raw: &str) -> anyhow::Result<i64> {
    let s0 = raw.trim();
    anyhow::ensure!(!s0.is_empty(), "empty input");

    let s = s0
        .to_ascii_lowercase()
        .replace(['_', ' '], "")
        .replace(',', ".");

    let (number, multiplier): (&str, f64) = if let Some(n) = s.strip_suffix("mhz") {
        (n, 1_000_000.0)
    } else if let Some(n) = s.strip_suffix("khz") {
        (n, 1_000.0)
    } else if let Some(n) = s.strip_suffix("hz") {
        (n, 1.0)
    } else {
        (s.as_str(), 1_000.0)
    };

    let v: f64 = number.trim().parse().context("parse number")?;
    anyhow::ensure!(v.is_finite() && v >= 0.0, "value must be >= 0");
    let hz = (v * multiplier).round();
    anyhow::ensure!(hz >= 0.0, "value must be >= 0");
    anyhow::ensure!(hz <= (i64::MAX as f64), "value too large");
    Ok(hz as i64)
}

fn format_khz(khz: f64) -> String {
    if khz.fract().abs() < 1e-9 {
        format!("{}", khz.round() as i64)
    } else {
        let s = format!("{khz:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn prompt_sample_rate_sps(label: &str, default_sps: i64) -> anyhow::Result<i64> {
    let default_str = format_ksps((default_sps as f64) / 1_000.0);
    loop {
        let raw = Text::new(&format!(
            "{label} (kS/s default; examples: 2.048msps, 2048ksps, 2048000sps)"
        ))
        .with_default(default_str.as_str())
        .prompt()
        .with_context(|| format!("prompt {label}"))?;
        match parse_sample_rate_to_sps(raw.as_str()) {
            Ok(sps) => return Ok(sps),
            Err(e) => ui::line(&format!("Invalid sample rate: {e}")),
        }
    }
}

fn prompt_audio_rate_hz(label: &str, default_hz: i64) -> anyhow::Result<i64> {
    let default_str = default_hz.to_string();
    loop {
        let raw = Text::new(&format!("{label} (Hz default; examples: 12000, 12ksps)"))
            .with_default(default_str.as_str())
            .prompt()
            .with_context(|| format!("prompt {label}"))?;
        match parse_rate_to_sps(raw.as_str(), 1.0) {
            Ok(sps) => return Ok(sps),
            Err(e) => ui::line(&format!("Invalid audio rate: {e}")),
        }
    }
}

fn prompt_passband_hz(label: &str, default_hz: i64) -> anyhow::Result<i64> {
    loop {
        let raw = Text::new(&format!("{label} (Hz; integer)"))
            .with_default(&default_hz.to_string())
            .prompt()
            .with_context(|| format!("prompt {label}"))?;
        let parsed = raw
            .trim()
            .parse::<i64>()
            .with_context(|| format!("parse {label}"))?;
        if parsed < 0 {
            ui::line("Value must be >= 0.");
            continue;
        }
        return Ok(parsed);
    }
}

fn parse_sample_rate_to_sps(raw: &str) -> anyhow::Result<i64> {
    parse_rate_to_sps(raw, 1_000.0)
}

fn parse_rate_to_sps(raw: &str, default_multiplier: f64) -> anyhow::Result<i64> {
    let s0 = raw.trim();
    anyhow::ensure!(!s0.is_empty(), "empty input");

    let s = s0
        .to_ascii_lowercase()
        .replace(['_', ' '], "")
        .replace(',', ".");

    let (number, multiplier): (&str, f64) = if let Some(n) = s.strip_suffix("msps") {
        (n, 1_000_000.0)
    } else if let Some(n) = s.strip_suffix("ksps") {
        (n, 1_000.0)
    } else if let Some(n) = s.strip_suffix("sps") {
        (n, 1.0)
    } else {
        (s.as_str(), default_multiplier)
    };

    let v: f64 = number.trim().parse().context("parse number")?;
    anyhow::ensure!(v.is_finite() && v > 0.0, "value must be > 0");
    let sps = (v * multiplier).round();
    anyhow::ensure!(sps <= (i64::MAX as f64), "value too large");
    Ok(sps as i64)
}

fn format_ksps(ksps: f64) -> String {
    if ksps.fract().abs() < 1e-9 {
        format!("{}", ksps.round() as i64)
    } else {
        let s = format!("{ksps:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(feature = "soapysdr")]
fn with_spinner<T, F>(label: &str, f: F) -> T
where
    F: FnOnce() -> T,
{
    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let label_str = label.to_string();
    let label_for_thread = label_str.clone();

    let handle = std::thread::spawn(move || {
        let frames = ['|', '/', '-', '\\'];
        let mut i = 0usize;
        while !done2.load(Ordering::Relaxed) {
            ui::overwrite_line(&format!("{label_for_thread} {} ", frames[i % frames.len()]));
            i = i.wrapping_add(1);
            std::thread::sleep(Duration::from_millis(80));
        }
    });

    let out = f();
    done.store(true, Ordering::Relaxed);
    let _ = handle.join();
    ui::overwrite_done(&format!("{label_str}... done"));
    out
}
