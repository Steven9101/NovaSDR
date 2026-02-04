use novasdr_core::config::load_from_files;
use std::{fs, path::PathBuf};

fn write_temp(name: &str, contents: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let nonce = format!("{}_{}_{}", std::process::id(), now, rand::random::<u32>());
    p.push(format!("novasdr_{nonce}_{name}"));
    fs::write(&p, contents).unwrap();
    p
}

#[test]
fn json_load_defaults_active_receiver_when_single_receiver() {
    let config = write_temp(
        "config.json",
        r#"{
  "server": { "port": 9002, "host": "0.0.0.0", "html_root": "frontend/dist/", "otherusers": 1, "threads": 1 },
  "websdr": { "name": "NovaSDR" },
  "limits": { "audio": 1, "waterfall": 1, "events": 1 }
}"#,
    );
    let receivers = write_temp(
        "receivers.json",
        r#"{
  "receivers": [
    {
      "id": "rx0",
      "name": "",
      "input": {
        "sps": 2048000,
        "frequency": 100900000,
        "signal": "iq",
        "driver": { "kind": "stdin", "format": "u8" }
      }
    }
  ]
}"#,
    );

    let cfg = load_from_files(&config, &receivers).unwrap();
    assert_eq!(cfg.active_receiver_id, "rx0");
    let rx = cfg.active_receiver().unwrap();
    assert_eq!(rx.id, "rx0");
    assert_eq!(rx.name, "rx0");
}

#[test]
fn json_load_requires_active_receiver_id_for_multiple_receivers() {
    let config = write_temp(
        "config.json",
        r#"{
  "server": { "port": 9002, "host": "0.0.0.0", "html_root": "frontend/dist/", "otherusers": 1, "threads": 1 },
  "websdr": { "name": "NovaSDR" },
  "limits": { "audio": 1, "waterfall": 1, "events": 1 }
}"#,
    );
    let receivers = write_temp(
        "receivers.json",
        r#"{
  "receivers": [
    { "id": "rx0", "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "stdin", "format": "u8" } } },
    { "id": "rx1", "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "soapysdr", "device": "driver=rtlsdr", "format": "cs16" } } }
  ]
}"#,
    );

    let err = load_from_files(&config, &receivers).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("active_receiver_id is required"),
        "unexpected error: {msg}"
    );
}

#[test]
fn json_load_rejects_multiple_stdin_receivers() {
    let config = write_temp(
        "config.json",
        r#"{
  "server": { "port": 9002, "host": "0.0.0.0", "html_root": "frontend/dist/", "otherusers": 1, "threads": 1 },
  "websdr": { "name": "NovaSDR" },
  "limits": { "audio": 1, "waterfall": 1, "events": 1 },
  "active_receiver_id": "rx0"
}"#,
    );
    let receivers = write_temp(
        "receivers.json",
        r#"{
  "receivers": [
    { "id": "rx0", "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "stdin", "format": "u8" } } },
    { "id": "rx1", "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "stdin", "format": "u8" } } }
  ]
}"#,
    );

    let err = load_from_files(&config, &receivers).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("only one enabled receiver may use input.driver.kind = \"stdin\""),
        "unexpected error: {msg}"
    );
}

#[test]
fn json_load_reject_disabled_stdin_receivers() {
    let config = write_temp(
        "config.json",
        r#"{
  "server": { "port": 9002, "host": "0.0.0.0", "html_root": "frontend/dist/", "otherusers": 1, "threads": 1 },
  "websdr": { "name": "NovaSDR" },
  "limits": { "audio": 1, "waterfall": 1, "events": 1 },
  "active_receiver_id": "rx0"
}"#,
    );
    let receivers = write_temp(
        "receivers.json",
        r#"{
  "receivers": [
    { "id": "rx0", "enabled": false, "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "stdin", "format": "u8" } } },
    { "id": "rx1", "enabled": true, "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "stdin", "format": "u8" } } }
  ]
}"#,
    );

    let err = load_from_files(&config, &receivers).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains(
            "active_receiver_id \"rx0\" not found amoung enabled receivers in receivers.json"
        ),
        "unexpected error: {msg}"
    );
}

#[test]
fn json_load_multiple_stdin_receivers() {
    let config = write_temp(
        "config.json",
        r#"{
  "server": { "port": 9002, "host": "0.0.0.0", "html_root": "frontend/dist/", "otherusers": 1, "threads": 1 },
  "websdr": { "name": "NovaSDR" },
  "limits": { "audio": 1, "waterfall": 1, "events": 1 },
  "active_receiver_id": "rx1"
}"#,
    );
    let receivers = write_temp(
        "receivers.json",
        r#"{
  "receivers": [
    { "id": "rx0", "enabled": false, "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "stdin", "format": "u8" } } },
    { "id": "rx1", "enabled": true, "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "stdin", "format": "u8" } } }
  ]
}"#,
    );

    let cfg = load_from_files(&config, &receivers).unwrap();
    assert_eq!(cfg.active_receiver_id, "rx1");
    let rx = cfg.active_receiver().unwrap();
    assert_eq!(rx.id, "rx1");
    assert_eq!(rx.name, "rx1");
}

#[test]
fn json_load_fifo_input() {
    let config = write_temp(
        "config.json",
        r#"{
  "server": { "port": 9002, "host": "0.0.0.0", "html_root": "frontend/dist/", "otherusers": 1, "threads": 1 },
  "websdr": { "name": "NovaSDR" },
  "limits": { "audio": 1, "waterfall": 1, "events": 1 },
  "active_receiver_id": "rx0"
}"#,
    );
    let receivers = write_temp(
        "receivers.json",
        r#"{
  "receivers": [
    { "id": "rx0", "input": { "sps": 2048000, "frequency": 100900000, "signal": "iq", "driver": { "kind": "fifo", "format": "cs16", "path": "somefile" } } }
  ]
}"#,
    );

    let cfg = load_from_files(&config, &receivers).unwrap();
    assert_eq!(cfg.active_receiver_id, "rx0");
}
