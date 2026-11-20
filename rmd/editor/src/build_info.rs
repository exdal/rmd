pub(crate) const APP_NAME: &str = "Rapid Mapping Device";

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) const GIT_SHORT_HASH: Option<&str> = option_env!("RMD_GIT_SHORT_HASH");

pub(crate) const VERSION_URL: Option<&str> = option_env!("RMD_VERSION_URL");

pub(crate) const COMMIT_URL: Option<&str> = option_env!("RMD_COMMIT_URL");

pub(crate) const REPOSITORY_URL: Option<&str> = option_env!("RMD_REPOSITORY_URL");

pub(crate) const CONTRIBUTORS: &str = include_str!(concat!(env!("OUT_DIR"), "/contributors.txt"));
