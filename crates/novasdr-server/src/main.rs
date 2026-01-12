mod app;
mod banner;
mod build_info;
mod cli;
mod dsp_runner;
mod input;
mod logging;
mod overlays;
mod registration;
mod setup;
mod shutdown;
mod state;
mod update_check;
mod ws;

use anyhow::Context;
use novasdr_core::config;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

fn receivers_file_has_receivers(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return true;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return true;
    };
    let Some(arr) = v.get("receivers").and_then(|v| v.as_array()) else {
        return true;
    };
    !arr.is_empty()
}

fn run_setup(args: &cli::Args, mode: setup::RunMode) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build setup runtime")?
        .block_on(setup::run(args, mode))
}

fn resolve_html_root(configured: &str) -> std::path::PathBuf {
    use std::path::PathBuf;
    let configured_path = PathBuf::from(configured);
    if configured_path.is_absolute() {
        return configured_path;
    }
    if configured_path.exists() {
        return configured_path;
    }

    let Ok(exe) = std::env::current_exe() else {
        return configured_path;
    };
    // current_exe() can return a symlink path (common when installed via /usr/local/bin -> /opt/...).
    // Prefer resolving the real binary path so relative html_root works in service environments.
    let exe = exe.canonicalize().unwrap_or(exe);
    let Some(exe_dir) = exe.parent() else {
        return configured_path;
    };
    let candidate = exe_dir.join(&configured_path);
    if candidate.exists() {
        return candidate;
    }

    configured_path
}

fn main() -> anyhow::Result<()> {
    use clap::parser::ValueSource;
    use clap::{CommandFactory, FromArgMatches};
    use std::path::PathBuf;

    let cmd = cli::Args::command();
    let matches = cmd.get_matches();
    let args = cli::Args::from_arg_matches(&matches).context("parse args")?;

    if matches!(
        args.command,
        Some(cli::Command::Setup) | Some(cli::Command::Configure)
    ) {
        return run_setup(&args, setup::RunMode::Explicit);
    }

    let config_source = matches.value_source("config");
    let receivers_source = matches.value_source("receivers");

    let config_provided = config_source == Some(ValueSource::CommandLine);
    let receivers_provided = receivers_source == Some(ValueSource::CommandLine);
    let config_is_default = config_source == Some(ValueSource::DefaultValue);
    let receivers_is_default = receivers_source == Some(ValueSource::DefaultValue);

    let mut config_path = args.config.clone();
    let mut receivers_path = args.receivers.clone();
    let mut using_legacy_default_paths = false;

    if config_is_default && receivers_is_default {
        let new_config_exists = config_path.exists();
        let new_receivers_exists = receivers_path.exists();
        if !new_config_exists && !new_receivers_exists {
            let legacy_config = PathBuf::from("config.json");
            let legacy_receivers = PathBuf::from("receivers.json");
            if legacy_config.exists() && legacy_receivers.exists() {
                config_path = legacy_config;
                receivers_path = legacy_receivers;
                using_legacy_default_paths = true;
            }
        }
    }

    let config_exists = config_path.exists();
    let receivers_exists = receivers_path.exists();
    let receivers_has_entries = receivers_exists && receivers_file_has_receivers(&receivers_path);
    let receivers_usable = receivers_exists && receivers_has_entries;
    if !config_exists || !receivers_usable {
        let interactive = std::io::stdin().is_terminal();
        if !interactive {
            anyhow::bail!(
                "missing config files: config={}, receivers={} (run `novasdr-server setup` in a terminal)",
                config_path.display(),
                receivers_path.display()
            );
        }

        let first_launch =
            !config_exists && !receivers_usable && !config_provided && !receivers_provided;

        if first_launch && cfg!(feature = "soapysdr") {
            return run_setup(&args, setup::RunMode::FirstLaunchSoapy);
        }

        if setup::ask_to_run_setup(&args, config_provided || receivers_provided)? {
            return run_setup(&args, setup::RunMode::Prompted);
        }

        anyhow::bail!(
            "missing config files: config={}, receivers={}",
            config_path.display(),
            receivers_path.display()
        );
    }

    let log_dir = if args.no_file_log {
        None
    } else {
        args.log_dir
            .clone()
            .or_else(|| Some(logging::default_log_dir()))
    };
    let log_cfg = logging::LoggingConfig {
        debug: args.debug,
        log_dir,
        log_file_prefix: "novasdr".to_string(),
    };
    let _log_guards = logging::init(&log_cfg)?;

    banner::log_startup_banner();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    if using_legacy_default_paths {
        tracing::warn!(
            config = %config_path.display(),
            receivers = %receivers_path.display(),
            "using legacy config paths (migrate to ./config/config.json and ./config/receivers.json)"
        );
    }

    let cfg = match config::load_from_files(&config_path, &receivers_path) {
        Ok(mut cfg) => {
            for r in cfg.receivers.iter_mut() {
                if r.input.audio_compression == config::AudioCompression::Flac {
                    tracing::warn!(
                        receiver_id = %r.id,
                        "audio_compression = \"flac\" was removed; treating it as \"adpcm\""
                    );
                    r.input.audio_compression = config::AudioCompression::Adpcm;
                }
            }
            Arc::new(cfg)
        }
        Err(e) => {
            let interactive = std::io::stdin().is_terminal();
            if interactive && setup::ask_to_run_setup_for_invalid_config(&args, &e)? {
                return run_setup(&args, setup::RunMode::Prompted);
            }
            return Err(e).context("load config");
        }
    };
    let resolved_html_root = resolve_html_root(cfg.server.html_root.as_str());
    tracing::info!(
        config = %config_path.display(),
        receivers = %receivers_path.display(),
        receiver_id = %cfg.active_receiver_id,
        receiver_count = cfg.receivers.len(),
        host = %cfg.server.host,
        port = cfg.server.port,
        html_root = %cfg.server.html_root,
        html_root_resolved = %resolved_html_root.display(),
        "config loaded"
    );
    for r in cfg.receivers.iter() {
        match r.input.accelerator {
            config::Accelerator::None => {}
            config::Accelerator::Clfft => {
                if !cfg!(feature = "clfft") {
                    anyhow::bail!(
                        "receiver {}: accelerator = \"clfft\" requires building novasdr-server with: cargo build -p novasdr-server --release --features clfft",
                        r.id
                    );
                }
                tracing::info!(receiver_id = %r.id, "accelerator: clfft");
            }
            config::Accelerator::Vkfft => {
                if !cfg!(feature = "vkfft") {
                    anyhow::bail!(
                        "receiver {}: accelerator = \"vkfft\" requires building novasdr-server with: cargo build -p novasdr-server --release --features vkfft",
                        r.id
                    );
                }
                tracing::info!(receiver_id = %r.id, "accelerator: vkfft");
            }
            config::Accelerator::Unsupported => {
                anyhow::bail!("receiver {}: unsupported accelerator configured", r.id);
            }
        }
        if r.input.waterfall_compression != config::WaterfallCompression::Zstd {
            anyhow::bail!(
                "receiver {}: only waterfall_compression = \"zstd\" is supported",
                r.id
            );
        }
        match &r.input.driver {
            config::InputDriver::Stdin { .. } => {}
            config::InputDriver::SoapySdr(_) => {
                if !cfg!(feature = "soapysdr") {
                    anyhow::bail!(
                        "receiver {}: input.driver.kind = \"soapysdr\" requires building novasdr-server with: cargo build -p novasdr-server --release --features soapysdr",
                        r.id
                    );
                }
            }
        }
    }

    let available_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let worker_threads = if cfg.server.threads == 0 {
        let auto_threads = if available_threads <= 2 {
            available_threads
        } else if available_threads <= 4 {
            2
        } else {
            (available_threads / 2).clamp(2, 8)
        };
        tracing::info!(
            available_threads,
            worker_threads = auto_threads,
            "tokio runtime configured (auto)"
        );
        auto_threads
    } else {
        let requested_threads = cfg.server.threads.max(1);
        let clamped_threads = requested_threads.min(available_threads).max(1);
        if clamped_threads != requested_threads {
            tracing::warn!(
                requested_threads,
                available_threads,
                worker_threads = clamped_threads,
                "server.threads exceeds available parallelism; clamping"
            );
        }
        tracing::info!(worker_threads = clamped_threads, "tokio runtime configured");
        clamped_threads
    }
    .max(1);

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .thread_name("novasdr-tokio")
        .build()
        .context("build tokio runtime")?
        .block_on(async move {
            let state = Arc::new(
                state::AppState::new(cfg.clone(), resolved_html_root).context("init app state")?,
            );
            let active = state.active_receiver_state();
            tracing::info!(
                receiver_id = %cfg.active_receiver_id,
                sps = active.rt.sps,
                fft_size = active.rt.fft_size,
                fft_result_size = active.rt.fft_result_size,
                is_real = active.rt.is_real,
                basefreq = active.rt.basefreq,
                total_bandwidth = active.rt.total_bandwidth,
                audio_max_fft_size = active.rt.audio_max_fft_size,
                min_waterfall_fft = active.rt.min_waterfall_fft,
                downsample_levels = active.rt.downsample_levels,
                "active receiver runtime derived"
            );

            let overlays =
                overlays::ensure_default_overlays(&config_path).context("ensure overlays")?;
            state::load_overlays_once(state.clone(), overlays.dir.clone()).await;
            state::spawn_marker_watcher(state.clone(), overlays.dir.clone());
            state::spawn_bands_watcher(state.clone(), overlays.dir);
            registration::spawn(state.clone());
            update_check::spawn(state.clone());
            dsp_runner::start(state.clone()).context("start DSP runner")?;

            app::serve(state).await
        })
}
