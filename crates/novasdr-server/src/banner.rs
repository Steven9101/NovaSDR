pub fn log_startup_banner() {
    let version = crate::build_info::version();
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let build = option_env!("NOVASDR_BUILD").unwrap_or("");
    let build_source = crate::build_info::build_source();
    let features = crate::build_info::features();
    let profile = crate::build_info::profile();
    let target = crate::build_info::target();
    let release_tag = crate::build_info::release_tag().unwrap_or("");
    let git_commit = crate::build_info::git_commit().unwrap_or("");
    let git_tag = crate::build_info::git_tag().unwrap_or("");
    let git_dirty = crate::build_info::git_dirty();

    tracing::info!(
        target: "novasdr_banner",
        version,
        os,
        arch,
        timestamp = %timestamp,
        build,
        build_source,
        features = %features,
        profile,
        target,
        release_tag,
        git_commit,
        git_tag,
        git_dirty,
        "startup"
    );
}
