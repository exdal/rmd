use std::{
    sync::mpsc::{Receiver, channel},
    thread,
    time::Duration,
};

use serde::Deserialize;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const REPOSITORY_URL: Option<&str> = option_env!("RMD_REPOSITORY_URL");

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

#[derive(Default)]
pub struct UpdateCheck {
    started: bool,
    receiver: Option<Receiver<Option<Release>>>,
    release: Option<Release>,
}

impl UpdateCheck {
    pub fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;

        let Some(repo) = REPOSITORY_URL.and_then(github_repo) else {
            return;
        };

        let url = format!("https://api.github.com/repos/{repo}/releases/latest");
        let (sender, receiver) = channel();
        self.receiver = Some(receiver);

        thread::spawn(move || {
            let release = match fetch_latest(&url) {
                Ok(latest) => is_newer(&latest.tag_name, CURRENT_VERSION).then_some(Release {
                    version: latest.tag_name,
                    url: latest.html_url,
                }),
                Err(error) => {
                    log::warn!("checking for updates: {error}");
                    None
                },
            };
            let _ = sender.send(release);
        });
    }

    pub fn poll(&mut self) -> Option<&Release> {
        if let Some(receiver) = &self.receiver
            && let Ok(release) = receiver.try_recv()
        {
            self.release = release;
            self.receiver = None;
        }

        self.release.as_ref()
    }
}

fn fetch_latest(url: &str) -> Result<GithubRelease, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into();

    agent
        .get(url)
        .header("User-Agent", concat!("rmd/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_json()
}

fn github_repo(url: &str) -> Option<&str> {
    let repo = url.strip_prefix("https://github.com/")?.trim_end_matches('/');
    let (owner, name) = repo.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/')).then_some(repo)
}

fn is_newer(tag: &str, current: &str) -> bool {
    let parse = |version: &str| semver::Version::parse(version.strip_prefix('v').unwrap_or(version)).ok();

    match (parse(tag), parse(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{github_repo, is_newer};

    #[test]
    fn only_github_repositories_are_checked() {
        assert_eq!(github_repo("https://github.com/exdal/dmed"), Some("exdal/dmed"));
        assert_eq!(github_repo("https://github.com/exdal/dmed/"), Some("exdal/dmed"));
        assert_eq!(github_repo("https://gitlab.com/exdal/dmed"), None);
        assert_eq!(github_repo("https://github.com/exdal"), None);
        assert_eq!(github_repo("https://github.com/exdal/dmed/tree/master"), None);
    }

    #[test]
    fn only_a_strictly_newer_release_is_offered() {
        assert!(is_newer("v1.8.0", "1.7.0"));
        assert!(is_newer("v1.7.1", "1.7.0"));
        assert!(!is_newer("v1.7.0", "1.7.0"));
        assert!(!is_newer("v1.6.0", "1.7.0"));
        assert!(!is_newer("nightly", "1.7.0"));
    }

    #[test]
    fn a_prerelease_ranks_below_its_final_release() {
        assert!(is_newer("v1.8.0-preview1", "1.7.0"));
        assert!(is_newer("v1.7.0", "1.7.0-preview1"));
        assert!(!is_newer("v1.7.0-preview1", "1.7.0"));
    }
}
