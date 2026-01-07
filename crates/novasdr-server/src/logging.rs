use anyhow::Context;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tracing::{field::Visit, Subscriber};
use tracing_subscriber::{filter::FilterFn, Layer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub struct LoggingGuards {
    _file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

#[derive(Debug, Clone)]
pub struct LoggingConfig {
    pub debug: bool,
    pub log_dir: Option<PathBuf>,
    pub log_file_prefix: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            debug: false,
            log_dir: None,
            log_file_prefix: "novasdr".to_string(),
        }
    }
}

pub fn init(cfg: &LoggingConfig) -> anyhow::Result<LoggingGuards> {
    let env_filter = if let Ok(v) = std::env::var("RUST_LOG") {
        EnvFilter::new(v)
    } else if cfg.debug {
        EnvFilter::new("info,novasdr_server=debug,novasdr_core=debug")
    } else {
        EnvFilter::new("info")
    };

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr)
        .with_filter(FilterFn::new(|meta| meta.target() != "novasdr_banner"));

    let banner_layer = BannerLayer::new();

    let (file_layer, file_guard) = match &cfg.log_dir {
        None => (None, None),
        Some(dir) => {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("create log dir {}", dir.display()))?;
            let appender = tracing_appender::rolling::daily(dir, &cfg.log_file_prefix);
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_ansi(false)
                .with_writer(writer)
                .with_filter(FilterFn::new(|meta| meta.target() != "novasdr_banner"));
            (Some(layer), Some(guard))
        }
    };

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(banner_layer)
        .with(stderr_layer);
    match file_layer {
        None => registry.init(),
        Some(layer) => registry.with(layer).init(),
    }

    std::panic::set_hook(Box::new(|panic_info| {
        tracing::error!(panic = %panic_info, "panic");
    }));

    Ok(LoggingGuards {
        _file_guard: file_guard,
    })
}

pub fn default_log_dir() -> PathBuf {
    Path::new("logs").to_path_buf()
}

struct BannerLayer {}

impl BannerLayer {
    fn new() -> Self {
        Self {}
    }
}

impl<S> Layer<S> for BannerLayer
where
    S: Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        match event.metadata().target() {
            "novasdr_banner" => {
                let mut v = BannerVisitor {
                    version: None,
                    os: None,
                    arch: None,
                    timestamp: None,
                    build: None,
                };
                event.record(&mut v);
                let version = v.version.as_deref().unwrap_or("unknown");
                let os = v.os.as_deref().unwrap_or(std::env::consts::OS);
                let arch = v.arch.as_deref().unwrap_or(std::env::consts::ARCH);
                let timestamp = v.timestamp.as_deref().unwrap_or("unknown timestamp");
                let build = v.build.as_deref().unwrap_or("");

                let mut line = format!("NovaSDR v{version} ({os}/{arch}) {timestamp}");
                let build = build.trim();
                if !build.is_empty() {
                    line.push_str(" build=");
                    line.push_str(build);
                }
                line.push('\n');
                write_stderr(line.as_bytes());
            }
            "novasdr_notice" => {
                let mut v = NoticeVisitor {
                    current: None,
                    latest: None,
                    url: None,
                };
                event.record(&mut v);
                let current = v.current.as_deref().unwrap_or("unknown");
                let latest = v.latest.as_deref().unwrap_or("unknown");
                let url = v.url.as_deref().unwrap_or("");

                let mut out = String::new();
                out.push('\n');
                out.push_str("========================================\n");
                out.push_str("UPDATE AVAILABLE\n");
                out.push_str("========================================\n");
                out.push_str(&format!("Installed: v{current}\n"));
                out.push_str(&format!("Latest:    v{latest}\n"));
                if !url.trim().is_empty() {
                    out.push_str(&format!("Release:   {url}\n"));
                }
                out.push_str("\n");
                out.push_str("This build will not auto-update for safety.\n");
                out.push_str("Download the official release and replace the binary.\n");
                out.push_str("If you built from source: git pull && cargo build -p novasdr-server --release\n");
                out.push_str("To disable this message: set updates.check_on_startup=false\n");
                out.push_str("========================================\n\n");
                write_stderr(out.as_bytes());
            }
            _ => {}
        }
    }
}

fn write_stderr(bytes: &[u8]) {
    let mut stderr = std::io::stderr().lock();
    if std::io::Write::flush(&mut stderr).is_err() {
        return;
    }
    let _ = std::io::Write::write_all(&mut stderr, bytes);
}

struct BannerVisitor {
    version: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    timestamp: Option<String>,
    build: Option<String>,
}

impl Visit for BannerVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "version" => self.version = Some(value.to_string()),
            "os" => self.os = Some(value.to_string()),
            "arch" => self.arch = Some(value.to_string()),
            "timestamp" => self.timestamp = Some(value.to_string()),
            "build" => self.build = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{value:?}");
        match field.name() {
            "version" if self.version.is_none() => self.version = Some(s),
            "os" if self.os.is_none() => self.os = Some(s),
            "arch" if self.arch.is_none() => self.arch = Some(s),
            "timestamp" if self.timestamp.is_none() => self.timestamp = Some(s),
            "build" if self.build.is_none() => self.build = Some(s),
            _ => {}
        }
    }
}

struct NoticeVisitor {
    current: Option<String>,
    latest: Option<String>,
    url: Option<String>,
}

impl Visit for NoticeVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "current" => self.current = Some(value.to_string()),
            "latest" => self.latest = Some(value.to_string()),
            "url" => self.url = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{value:?}");
        match field.name() {
            "current" if self.current.is_none() => self.current = Some(s),
            "latest" if self.latest.is_none() => self.latest = Some(s),
            "url" if self.url.is_none() => self.url = Some(s),
            _ => {}
        }
    }
}
