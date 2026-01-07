use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=NOVASDR_BUILD_SOURCE");
    println!("cargo:rerun-if-env-changed=NOVASDR_RELEASE_TAG");
    println!("cargo:rerun-if-env-changed=NOVASDR_BUILD");

    if let Ok(profile) = std::env::var("PROFILE") {
        println!("cargo:rustc-env=NOVASDR_PROFILE={profile}");
    }
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=NOVASDR_TARGET={target}");
    }

    let mut enabled_features = Vec::new();
    for (feature_env, feature_name) in [
        ("CARGO_FEATURE_CLFFT", "clfft"),
        ("CARGO_FEATURE_VKFFT", "vkfft"),
        ("CARGO_FEATURE_SOAPYSDR", "soapysdr"),
    ] {
        if std::env::var_os(feature_env).is_some() {
            enabled_features.push(feature_name);
        }
    }
    println!(
        "cargo:rustc-env=NOVASDR_FEATURES={}",
        enabled_features.join(",")
    );

    if let Ok(source) = std::env::var("NOVASDR_BUILD_SOURCE") {
        println!("cargo:rustc-env=NOVASDR_BUILD_SOURCE={source}");
    }
    if let Ok(tag) = std::env::var("NOVASDR_RELEASE_TAG") {
        println!("cargo:rustc-env=NOVASDR_RELEASE_TAG={tag}");
    }

    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let git_commit = run_git(["rev-parse", "--short=12", "HEAD"]);
    if let Some(commit) = git_commit.as_deref() {
        println!("cargo:rustc-env=NOVASDR_GIT_COMMIT={commit}");
    }
    let git_tag = run_git(["describe", "--tags", "--exact-match"]);
    if let Some(tag) = git_tag.as_deref() {
        println!("cargo:rustc-env=NOVASDR_GIT_TAG={tag}");
    }
    let git_dirty = run_git(["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty());
    println!("cargo:rustc-env=NOVASDR_GIT_DIRTY={git_dirty}");

    if std::env::var_os("NOVASDR_BUILD_SOURCE").is_none() {
        let inferred = if git_commit.is_some() {
            "git"
        } else {
            "unknown"
        };
        println!("cargo:rustc-env=NOVASDR_BUILD_SOURCE={inferred}");
    }
}

fn run_git<const N: usize>(args: [&str; N]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    Some(s.trim().to_string())
}
