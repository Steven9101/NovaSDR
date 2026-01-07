use std::borrow::Cow;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn build_source() -> &'static str {
    option_env!("NOVASDR_BUILD_SOURCE").unwrap_or("unknown")
}

pub fn release_tag() -> Option<&'static str> {
    option_env!("NOVASDR_RELEASE_TAG")
}

pub fn features() -> Cow<'static, str> {
    Cow::Borrowed(option_env!("NOVASDR_FEATURES").unwrap_or(""))
}

pub fn profile() -> &'static str {
    option_env!("NOVASDR_PROFILE").unwrap_or("release")
}

pub fn target() -> &'static str {
    option_env!("NOVASDR_TARGET").unwrap_or("")
}

pub fn git_commit() -> Option<&'static str> {
    option_env!("NOVASDR_GIT_COMMIT")
}

pub fn git_tag() -> Option<&'static str> {
    option_env!("NOVASDR_GIT_TAG")
}

pub fn git_dirty() -> Option<bool> {
    option_env!("NOVASDR_GIT_DIRTY").and_then(|s| match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

