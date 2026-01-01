pub fn log_startup_banner() {
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let build = option_env!("NOVASDR_BUILD").unwrap_or("");

    tracing::info!(
        target: "novasdr_banner",
        version,
        os,
        arch,
        timestamp = %timestamp,
        build,
        "startup"
    );
}
