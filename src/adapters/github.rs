//! Release source backed by GitHub Releases.

use std::time::Duration;

use crate::app::update::{Release, ReleaseSource, UpdateError, parse_tag};
use crate::domain::Checksums;

/// Where vump publishes its own binaries.
const REPOSITORY: &str = "codehacks-io/vump";

/// Requests are given a bound so that an unreachable network fails rather than
/// hanging a command the user is waiting on.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Releases fetched in one request. Comfortably more than this project will
/// publish between prunings, and one page keeps the command a single call.
const PAGE_SIZE: u32 = 100;

/// The asset carrying digests for every binary in a release.
///
/// The name is shared with the release workflow and the CI action; changing it
/// in one place without the others breaks verification everywhere.
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS";

/// An upper bound on a downloaded artifact, so a hostile or broken response
/// cannot exhaust memory.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

/// Reads releases from the GitHub API and replaces the running binary.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitHubReleases;

impl GitHubReleases {
    /// Creates a source pointing at vump's own repository.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Fetches one release asset.
    fn download(release: &Release, asset: &str) -> Result<Vec<u8>, UpdateError> {
        let url = format!(
            "https://github.com/{REPOSITORY}/releases/download/{}/{asset}",
            release.tag
        );

        let fail = |detail: String| UpdateError::InstallFailed {
            asset: asset.to_owned(),
            detail,
        };

        let mut response = Self::agent()
            .get(&url)
            .call()
            .map_err(|e| fail(e.to_string()))?;

        response
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()
            .map_err(|e| fail(e.to_string()))
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("vump/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    }
}

impl ReleaseSource for GitHubReleases {
    fn list(&self) -> Result<Vec<Release>, UpdateError> {
        // The whole list is read rather than the "latest" endpoint, because
        // that endpoint answers only one channel's question. Maturity is
        // derived from each version itself, which also keeps the answer
        // consistent with releases whose pre-release flag was set by hand.
        let url =
            format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page={PAGE_SIZE}");

        let mut response = Self::agent()
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| UpdateError::Unreachable {
                detail: e.to_string(),
            })?;

        let body: serde_json::Value =
            response
                .body_mut()
                .read_json()
                .map_err(|e| UpdateError::Unreachable {
                    detail: e.to_string(),
                })?;

        let entries = body.as_array().ok_or_else(|| UpdateError::Unreachable {
            detail: "response was not a list of releases".to_owned(),
        })?;

        // Drafts are not installable, and a tag that is not a version is
        // skipped rather than treated as fatal: one unrelated tag in the
        // repository must not hide every release.
        Ok(entries
            .iter()
            .filter(|entry| {
                !entry
                    .get("draft")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            })
            .filter_map(|entry| entry.get("tag_name").and_then(serde_json::Value::as_str))
            .filter_map(parse_tag)
            .collect())
    }

    fn checksums(&self, release: &Release) -> Result<Option<Checksums>, UpdateError> {
        let text = match Self::download(release, CHECKSUMS_ASSET) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            // A release published before checksums existed simply has no such
            // asset. That is reported as absence, and refused by the caller,
            // rather than mistaken for a network failure.
            Err(UpdateError::InstallFailed { .. }) => return Ok(None),
            Err(other) => return Err(other),
        };

        Ok(Some(Checksums::parse(&text)))
    }

    fn install(
        &self,
        release: &Release,
        asset: &str,
        expected: &Checksums,
    ) -> Result<(), UpdateError> {
        let fail = |detail: String| UpdateError::InstallFailed {
            asset: asset.to_owned(),
            detail,
        };

        let bytes = Self::download(release, asset)?;

        // Verified before anything touches the filesystem: a binary that fails
        // this must never exist on disk, let alone be executed.
        expected.verify(asset, &bytes)?;

        // The replacement is staged next to the running binary so that the
        // final move is within one filesystem, and therefore atomic.
        let exe = std::env::current_exe().map_err(|e| fail(e.to_string()))?;
        let directory = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
        let staged = directory.join(format!(".{asset}.incoming"));

        std::fs::write(&staged, &bytes).map_err(|e| fail(e.to_string()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| fail(e.to_string()))?;
        }

        // Replacing a running executable differs sharply between platforms:
        // Unix can rename over an open file, Windows must move the original
        // aside first. That is delegated rather than reimplemented.
        let result = self_replace::self_replace(&staged).map_err(|e| fail(e.to_string()));

        // Best-effort: a leftover staging file is untidy but harmless, and
        // must not mask the outcome of the replacement itself.
        let _ = std::fs::remove_file(&staged);

        result
    }
}
