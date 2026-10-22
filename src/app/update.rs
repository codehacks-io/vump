//! Inspecting published releases and replacing the running binary.
//!
//! Selection, filtering and the decision to act are pure and tested here.
//! Fetching and replacing sit behind the [`ReleaseSource`] port, so every
//! interesting question — which releases qualify, which is newest, is acting on
//! it an upgrade at all — is exercised without a network.

use std::fmt;
use std::str::FromStr;

use semver::Version;
use thiserror::Error;

use crate::domain::PreLabel;
use crate::domain::checksum::{ChecksumError, Checksums};

/// Where releases are published and how the running binary is replaced.
pub trait ReleaseSource {
    /// Lists published releases, in no particular order.
    ///
    /// # Errors
    ///
    /// Returns [`UpdateError`] when releases cannot be listed.
    fn list(&self) -> Result<Vec<Release>, UpdateError>;

    /// The digests published alongside `release`.
    ///
    /// Returns `None` when the release publishes none, which is refused rather
    /// than tolerated — see [`update`].
    ///
    /// # Errors
    ///
    /// Returns [`UpdateError`] when the listing cannot be fetched or read.
    fn checksums(&self, release: &Release) -> Result<Option<Checksums>, UpdateError>;

    /// Downloads `asset` from `release` and replaces the running binary,
    /// installing nothing unless the contents match `expected`.
    ///
    /// # Errors
    ///
    /// Returns [`UpdateError`] when the download fails, the contents do not
    /// match, or the replacement fails.
    fn install(
        &self,
        release: &Release,
        asset: &str,
        expected: &Checksums,
    ) -> Result<(), UpdateError>;
}

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag as published, which may carry a leading `v`. It forms the
    /// download URL, so it is kept verbatim rather than normalized.
    pub tag: String,
    /// The version parsed from the tag.
    pub version: Version,
}

/// The least mature kind of release that may be offered.
///
/// This is a floor, not an exact match: asking for [`Channel::Rc`] accepts
/// release candidates *and* stable releases, because a stable release is
/// strictly more mature than the candidates that preceded it.
///
/// A floor is necessary because semver compares the version core before the
/// pre-release: `1.1.0-alpha.0` outranks `1.0.0-rc.5`. Simply taking the
/// newest pre-release would move someone tracking release candidates onto the
/// next minor's first alpha — a version upgrade but a stability downgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    /// Only releases with no pre-release suffix.
    #[default]
    Stable,
    /// Release candidates and stable releases.
    Rc,
    /// Betas and anything more mature.
    Beta,
    /// Any recognized release, including alphas.
    Alpha,
}

impl Channel {
    /// Whether a version is mature enough for this channel.
    ///
    /// A pre-release vump does not recognize is never offered: without a rank
    /// there is no way to judge it against the floor.
    #[must_use]
    pub fn accepts(self, version: &Version) -> bool {
        if version.pre.is_empty() {
            return true;
        }

        let Some(floor) = self.floor() else {
            return false;
        };

        label_of(version).is_some_and(|label| label >= floor)
    }

    /// The least mature pre-release label accepted, or `None` for stable only.
    fn floor(self) -> Option<PreLabel> {
        match self {
            Self::Stable => None,
            Self::Rc => Some(PreLabel::Rc),
            Self::Beta => Some(PreLabel::Beta),
            Self::Alpha => Some(PreLabel::Alpha),
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stable => "stable",
            Self::Rc => "rc",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        })
    }
}

impl FromStr for Channel {
    type Err = UpdateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stable" => Ok(Self::Stable),
            "rc" => Ok(Self::Rc),
            "beta" => Ok(Self::Beta),
            "alpha" => Ok(Self::Alpha),
            other => Err(UpdateError::UnknownChannel(other.to_owned())),
        }
    }
}

/// Reads the pre-release label of a version, if it carries a recognized one.
fn label_of(version: &Version) -> Option<PreLabel> {
    let text = version.pre.as_str();
    let name = text.split_once('.').map_or(text, |(name, _)| name);
    name.parse().ok()
}

/// Why an update could not be performed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UpdateError {
    /// Releases could not be reached or understood.
    #[error("cannot list releases: {detail}")]
    Unreachable {
        /// Underlying error detail.
        detail: String,
    },

    /// This platform has no published binary.
    #[error("no published binary for {os} {arch}")]
    UnsupportedPlatform {
        /// Operating system.
        os: String,
        /// Architecture.
        arch: String,
    },

    /// An explicitly requested version is not published.
    #[error("no release {requested}; published releases are: {available}")]
    NoSuchRelease {
        /// The version asked for.
        requested: Version,
        /// Comma-separated list of what does exist.
        available: String,
    },

    /// A channel name outside the supported set.
    #[error("unknown channel {0:?}; expected stable, rc, beta, or alpha")]
    UnknownChannel(String),

    /// The download or replacement failed.
    #[error("cannot install {asset}: {detail}")]
    InstallFailed {
        /// Asset being installed.
        asset: String,
        /// Underlying error detail.
        detail: String,
    },

    /// The release publishes nothing to verify its binaries against.
    #[error(
        "release {version} publishes no checksums, so its binaries cannot be verified; refusing to install"
    )]
    Unverifiable {
        /// The release that cannot be trusted.
        version: Version,
    },

    /// What arrived is not what was published.
    #[error("{0}")]
    Corrupt(#[from] ChecksumError),
}

/// What an update or status run concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// The running version is the newest in the channel.
    UpToDate {
        /// The version in use.
        current: Version,
    },
    /// A newer release exists and was not installed.
    Available {
        /// The version in use.
        current: Version,
        /// The newer published version.
        latest: Version,
    },
    /// A release was installed.
    ///
    /// `installed` may be older than `previous` when a version was named
    /// explicitly, which is what a rollback is.
    Installed {
        /// The version replaced.
        previous: Version,
        /// The version now in use.
        installed: Version,
    },
    /// The running version is newer than anything published in the channel.
    ///
    /// Normal for a development build, and never acted upon.
    Ahead {
        /// The version in use.
        current: Version,
        /// The newest published version.
        latest: Version,
    },
    /// The channel has no releases at all.
    NoneAvailable {
        /// The version in use.
        current: Version,
        /// The channel that was searched.
        channel: Channel,
    },
}

/// The releases a channel offers, newest first, alongside the running version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Matching releases, newest first.
    pub releases: Vec<Release>,
    /// The version in use, so a renderer can mark it.
    pub current: Version,
}

/// Reports whether a newer release exists, without installing anything.
///
/// # Errors
///
/// Returns [`UpdateError`] when releases cannot be listed.
pub fn status(
    source: &dyn ReleaseSource,
    current: &Version,
    channel: Channel,
) -> Result<UpdateOutcome, UpdateError> {
    let releases = source.list()?;
    Ok(compare(current, newest(&releases, channel), channel))
}

/// Lists the releases a channel offers, newest first.
///
/// # Errors
///
/// Returns [`UpdateError`] when releases cannot be listed.
pub fn list(
    source: &dyn ReleaseSource,
    current: &Version,
    channel: Channel,
) -> Result<Listing, UpdateError> {
    let mut releases: Vec<Release> = source
        .list()?
        .into_iter()
        .filter(|r| channel.accepts(&r.version))
        .collect();

    releases.sort_by(|a, b| b.version.cmp(&a.version));

    Ok(Listing {
        releases,
        current: current.clone(),
    })
}

/// Installs a release: the newest in `channel`, or `requested` when one is
/// named.
///
/// Naming a version installs exactly that one, whether or not it is newer and
/// whether or not the channel would have offered it. That is what makes a
/// rollback expressible, and it is safe to allow because the caller had to
/// write the version out.
///
/// # Errors
///
/// Returns [`UpdateError`] when releases cannot be listed, a requested version
/// does not exist, this platform has no published binary, or installation
/// fails.
pub fn update(
    source: &dyn ReleaseSource,
    current: &Version,
    target: (&str, &str),
    channel: Channel,
    requested: Option<&Version>,
) -> Result<UpdateOutcome, UpdateError> {
    let releases = source.list()?;

    let chosen =
        match requested {
            Some(wanted) => Some(releases.iter().find(|r| r.version == *wanted).ok_or_else(
                || UpdateError::NoSuchRelease {
                    requested: wanted.clone(),
                    available: summarize(&releases),
                },
            )?),
            None => newest(&releases, channel),
        };

    let Some(release) = chosen else {
        return Ok(UpdateOutcome::NoneAvailable {
            current: current.clone(),
            channel,
        });
    };

    // Without an explicit request, only a genuine upgrade is acted upon.
    if requested.is_none() {
        match compare(current, Some(release), channel) {
            UpdateOutcome::Available { .. } => {}
            settled => return Ok(settled),
        }
    } else if release.version == *current {
        return Ok(UpdateOutcome::UpToDate {
            current: current.clone(),
        });
    }

    let (os, arch) = target;
    let asset = asset_name(os, arch).ok_or_else(|| UpdateError::UnsupportedPlatform {
        os: os.to_owned(),
        arch: arch.to_owned(),
    })?;

    // Nothing is installed that cannot be checked first. A release without
    // published checksums is refused outright rather than installed with a
    // warning: a warning on the highest-privilege path in the tool is not a
    // safeguard, and the whole point of publishing digests is that an
    // unverified binary never runs.
    let checksums = source
        .checksums(release)?
        .filter(|sums| !sums.is_empty())
        .ok_or_else(|| UpdateError::Unverifiable {
            version: release.version.clone(),
        })?;

    source.install(release, &asset, &checksums)?;

    Ok(UpdateOutcome::Installed {
        previous: current.clone(),
        installed: release.version.clone(),
    })
}

/// The newest release a channel accepts.
fn newest(releases: &[Release], channel: Channel) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| channel.accepts(&r.version))
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Judges the running version against the newest release on offer.
fn compare(current: &Version, newest: Option<&Release>, channel: Channel) -> UpdateOutcome {
    let Some(release) = newest else {
        return UpdateOutcome::NoneAvailable {
            current: current.clone(),
            channel,
        };
    };

    match release.version.cmp(current) {
        std::cmp::Ordering::Equal => UpdateOutcome::UpToDate {
            current: current.clone(),
        },
        std::cmp::Ordering::Less => UpdateOutcome::Ahead {
            current: current.clone(),
            latest: release.version.clone(),
        },
        std::cmp::Ordering::Greater => UpdateOutcome::Available {
            current: current.clone(),
            latest: release.version.clone(),
        },
    }
}

/// A short, newest-first list of published versions, for diagnostics.
fn summarize(releases: &[Release]) -> String {
    let mut versions: Vec<&Version> = releases.iter().map(|r| &r.version).collect();
    versions.sort_by(|a, b| b.cmp(a));
    versions
        .iter()
        .take(5)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The release asset for a platform.
///
/// These names are a contract shared with the release workflow and the CI
/// action; changing one without the others breaks installation.
///
/// macOS publishes only a universal binary, which is also the only notarized
/// artifact, so both architectures resolve to it.
#[must_use]
pub fn asset_name(os: &str, arch: &str) -> Option<String> {
    match (os, arch) {
        ("macos", _) => Some("vump-darwin-universal".to_owned()),
        ("linux", "x86_64") => Some("vump-linux-amd64".to_owned()),
        ("linux", "aarch64") => Some("vump-linux-arm64".to_owned()),
        ("windows", "x86_64") => Some("vump-windows-amd64.exe".to_owned()),
        ("windows", "aarch64") => Some("vump-windows-arm64.exe".to_owned()),
        _ => None,
    }
}

/// Interprets a published tag, which conventionally carries a leading `v`.
///
/// Returns `None` for a tag that is not a version, so that an unrelated tag in
/// the repository does not prevent the rest from being read.
#[must_use]
pub fn parse_tag(tag: &str) -> Option<Release> {
    let version = tag.strip_prefix('v').unwrap_or(tag).parse().ok()?;
    Some(Release {
        tag: tag.to_owned(),
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn v(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn releases(tags: &[&str]) -> Vec<Release> {
        tags.iter().filter_map(|t| parse_tag(t)).collect()
    }

    /// The digest every fake release publishes for its binaries.
    const DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// A release source over a fixed set of tags that records installs.
    struct Fake {
        published: Vec<Release>,
        installed: RefCell<Vec<String>>,
        /// Releases that publish no checksums, by tag.
        unverifiable: Vec<String>,
    }

    impl Fake {
        fn with(tags: &[&str]) -> Self {
            Self {
                published: releases(tags),
                installed: RefCell::new(Vec::new()),
                unverifiable: Vec::new(),
            }
        }

        /// Marks a release as predating published checksums.
        fn without_checksums(mut self, tag: &str) -> Self {
            self.unverifiable.push(tag.to_owned());
            self
        }

        fn installs(&self) -> Vec<String> {
            self.installed.borrow().clone()
        }
    }

    impl ReleaseSource for Fake {
        fn list(&self) -> Result<Vec<Release>, UpdateError> {
            Ok(self.published.clone())
        }

        fn checksums(&self, release: &Release) -> Result<Option<Checksums>, UpdateError> {
            if self.unverifiable.contains(&release.tag) {
                return Ok(None);
            }
            Ok(Some(Checksums::parse(&format!(
                "{DIGEST}  vump-linux-amd64\n"
            ))))
        }

        fn install(
            &self,
            release: &Release,
            asset: &str,
            expected: &Checksums,
        ) -> Result<(), UpdateError> {
            // The real adapter verifies before replacing; the fake asserts the
            // digest it was handed actually covers the asset, so a use case
            // that passed the wrong listing would be caught here.
            assert!(
                expected.get(asset).is_some(),
                "install was handed checksums that do not cover {asset}"
            );
            self.installed.borrow_mut().push(release.tag.clone());
            Ok(())
        }
    }

    const LINUX: (&str, &str) = ("linux", "x86_64");

    // ─── Channel semantics ───────────────────────────────────────────────────

    #[test]
    fn stable_accepts_only_finished_releases() {
        let c = Channel::Stable;
        assert!(c.accepts(&v("1.0.0")));
        assert!(!c.accepts(&v("1.0.0-rc.1")));
        assert!(!c.accepts(&v("1.0.0-alpha.0")));
    }

    #[test]
    fn a_channel_is_a_floor_not_an_exact_match() {
        // Asking for release candidates must not exclude finished releases:
        // a stable release is strictly more mature than the rc before it.
        let c = Channel::Rc;
        assert!(c.accepts(&v("1.0.0")));
        assert!(c.accepts(&v("1.0.0-rc.1")));
        assert!(!c.accepts(&v("1.0.0-beta.9")));
        assert!(!c.accepts(&v("1.0.0-alpha.9")));
    }

    #[test]
    fn alpha_accepts_every_recognized_release() {
        let c = Channel::Alpha;
        for text in ["2.0.0", "1.0.0-rc.1", "1.0.0-beta.0", "1.0.0-alpha.0"] {
            assert!(c.accepts(&v(text)), "{text}");
        }
    }

    #[test]
    fn an_unrecognized_pre_release_is_never_offered() {
        // Without a rank there is no way to judge it against a floor.
        for channel in [Channel::Stable, Channel::Rc, Channel::Beta, Channel::Alpha] {
            assert!(
                !channel.accepts(&v("1.0.0-nightly.4")),
                "{channel} must not accept an unrankable pre-release"
            );
        }
    }

    #[test]
    fn a_channel_floor_outranks_semver_ordering() {
        // The case the floor exists for: semver compares the version core
        // first, so 1.1.0-alpha.0 outranks 1.0.0-rc.5 numerically. Someone
        // tracking release candidates must not be moved onto it.
        assert!(v("1.1.0-alpha.0") > v("1.0.0-rc.5"));

        let published = releases(&["v1.0.0-rc.5", "v1.1.0-alpha.0"]);

        let for_rc = newest(&published, Channel::Rc).unwrap();
        assert_eq!(for_rc.version, v("1.0.0-rc.5"));

        let for_alpha = newest(&published, Channel::Alpha).unwrap();
        assert_eq!(for_alpha.version, v("1.1.0-alpha.0"));
    }

    #[test]
    fn channels_round_trip_through_text() {
        for channel in [Channel::Stable, Channel::Rc, Channel::Beta, Channel::Alpha] {
            assert_eq!(channel.to_string().parse::<Channel>().unwrap(), channel);
        }
        assert!("nightly".parse::<Channel>().is_err());
    }

    // ─── status ──────────────────────────────────────────────────────────────

    #[test]
    fn status_reports_a_newer_release_without_installing() {
        let source = Fake::with(&["v1.0.0", "v1.1.0"]);
        let outcome = status(&source, &v("1.0.0"), Channel::Stable).unwrap();

        assert_eq!(
            outcome,
            UpdateOutcome::Available {
                current: v("1.0.0"),
                latest: v("1.1.0"),
            }
        );
        assert_eq!(source.installs(), Vec::<String>::new());
    }

    #[test]
    fn status_ignores_releases_below_the_channel_floor() {
        let source = Fake::with(&["v1.0.0", "v2.0.0-alpha.0"]);

        // On stable, the alpha is invisible.
        assert_eq!(
            status(&source, &v("1.0.0"), Channel::Stable).unwrap(),
            UpdateOutcome::UpToDate {
                current: v("1.0.0")
            }
        );

        // Opting in surfaces it.
        assert_eq!(
            status(&source, &v("1.0.0"), Channel::Alpha).unwrap(),
            UpdateOutcome::Available {
                current: v("1.0.0"),
                latest: v("2.0.0-alpha.0"),
            }
        );
    }

    #[test]
    fn a_channel_with_no_releases_says_so() {
        // Every published release is a pre-release, so stable offers nothing.
        let source = Fake::with(&["v0.1.0-alpha.9", "v0.2.0-alpha.2"]);
        let outcome = status(&source, &v("0.2.0-alpha.2"), Channel::Stable).unwrap();

        assert_eq!(
            outcome,
            UpdateOutcome::NoneAvailable {
                current: v("0.2.0-alpha.2"),
                channel: Channel::Stable,
            }
        );
    }

    // ─── update ──────────────────────────────────────────────────────────────

    #[test]
    fn installs_the_newest_release_in_the_channel() {
        let source = Fake::with(&["v1.0.0", "v1.1.0", "v2.0.0-beta.0"]);
        let outcome = update(&source, &v("1.0.0"), LINUX, Channel::Stable, None).unwrap();

        assert_eq!(
            outcome,
            UpdateOutcome::Installed {
                previous: v("1.0.0"),
                installed: v("1.1.0"),
            }
        );
        assert_eq!(source.installs(), ["v1.1.0"]);
    }

    #[test]
    fn never_downgrades_without_being_asked() {
        let source = Fake::with(&["v1.0.0"]);
        let outcome = update(&source, &v("1.4.0"), LINUX, Channel::Stable, None).unwrap();

        assert_eq!(
            outcome,
            UpdateOutcome::Ahead {
                current: v("1.4.0"),
                latest: v("1.0.0"),
            }
        );
        assert_eq!(source.installs(), Vec::<String>::new());
    }

    #[test]
    fn an_explicit_version_installs_even_when_older() {
        // This is what a rollback is. The caller wrote the version out, so the
        // downgrade guard would only be in the way.
        let source = Fake::with(&["v1.0.0", "v1.1.0", "v1.2.0"]);
        let outcome = update(
            &source,
            &v("1.2.0"),
            LINUX,
            Channel::Stable,
            Some(&v("1.0.0")),
        )
        .unwrap();

        assert_eq!(
            outcome,
            UpdateOutcome::Installed {
                previous: v("1.2.0"),
                installed: v("1.0.0"),
            }
        );
        assert_eq!(source.installs(), ["v1.0.0"]);
    }

    #[test]
    fn an_explicit_version_ignores_the_channel() {
        // Naming a pre-release is consent enough; the floor is for automatic
        // selection, not for overriding what was asked for.
        let source = Fake::with(&["v1.0.0", "v2.0.0-alpha.0"]);
        let outcome = update(
            &source,
            &v("1.0.0"),
            LINUX,
            Channel::Stable,
            Some(&v("2.0.0-alpha.0")),
        )
        .unwrap();

        assert!(matches!(outcome, UpdateOutcome::Installed { .. }));
        assert_eq!(source.installs(), ["v2.0.0-alpha.0"]);
    }

    #[test]
    fn requesting_the_running_version_does_nothing() {
        let source = Fake::with(&["v1.0.0"]);
        let outcome = update(
            &source,
            &v("1.0.0"),
            LINUX,
            Channel::Stable,
            Some(&v("1.0.0")),
        )
        .unwrap();

        assert_eq!(
            outcome,
            UpdateOutcome::UpToDate {
                current: v("1.0.0")
            }
        );
        assert_eq!(source.installs(), Vec::<String>::new());
    }

    #[test]
    fn an_unpublished_version_is_refused_with_what_does_exist() {
        let source = Fake::with(&["v1.0.0", "v1.1.0"]);
        let err = update(
            &source,
            &v("1.0.0"),
            LINUX,
            Channel::Stable,
            Some(&v("9.9.9")),
        )
        .unwrap_err();

        let UpdateError::NoSuchRelease { available, .. } = err else {
            panic!("expected NoSuchRelease, got {err:?}");
        };
        assert!(available.contains("1.1.0"), "{available}");
        assert_eq!(source.installs(), Vec::<String>::new());
    }

    #[test]
    fn a_release_without_published_checksums_is_refused() {
        // Refusing rather than warning: a warning on the path that downloads
        // and then executes a binary is not a safeguard.
        let source = Fake::with(&["v1.0.0", "v2.0.0"]).without_checksums("v2.0.0");

        let err = update(&source, &v("1.0.0"), LINUX, Channel::Stable, None).unwrap_err();

        assert!(matches!(err, UpdateError::Unverifiable { .. }));
        assert!(
            source.installs().is_empty(),
            "nothing may be installed from a release that cannot be verified"
        );
    }

    #[test]
    fn an_explicitly_requested_release_is_verified_too() {
        // Naming a version is consent to install it, not consent to skip
        // checking what arrives.
        let source = Fake::with(&["v1.0.0", "v2.0.0"]).without_checksums("v1.0.0");

        let err = update(
            &source,
            &v("2.0.0"),
            LINUX,
            Channel::Stable,
            Some(&v("1.0.0")),
        )
        .unwrap_err();

        assert!(matches!(err, UpdateError::Unverifiable { .. }));
        assert_eq!(source.installs(), Vec::<String>::new());
    }

    #[test]
    fn status_and_list_do_not_require_checksums() {
        // Neither downloads anything, so neither has anything to verify.
        let source = Fake::with(&["v1.0.0", "v2.0.0"])
            .without_checksums("v1.0.0")
            .without_checksums("v2.0.0");

        assert!(status(&source, &v("1.0.0"), Channel::Stable).is_ok());
        assert!(list(&source, &v("1.0.0"), Channel::Stable).is_ok());
    }

    #[test]
    fn an_unsupported_platform_is_reported_before_downloading() {
        let source = Fake::with(&["v2.0.0"]);
        let err = update(
            &source,
            &v("1.0.0"),
            ("freebsd", "x86_64"),
            Channel::Stable,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, UpdateError::UnsupportedPlatform { .. }));
        assert_eq!(source.installs(), Vec::<String>::new());
    }

    // ─── list ────────────────────────────────────────────────────────────────

    #[test]
    fn lists_matching_releases_newest_first() {
        let source = Fake::with(&["v1.0.0", "v1.2.0", "v1.1.0"]);
        let listing = list(&source, &v("1.1.0"), Channel::Stable).unwrap();

        let versions: Vec<String> = listing
            .releases
            .iter()
            .map(|r| r.version.to_string())
            .collect();
        assert_eq!(versions, ["1.2.0", "1.1.0", "1.0.0"]);
        assert_eq!(listing.current, v("1.1.0"));
    }

    #[test]
    fn listing_respects_the_channel_floor() {
        let source = Fake::with(&["v1.0.0", "v1.1.0-rc.0", "v1.2.0-alpha.0"]);

        let stable = list(&source, &v("1.0.0"), Channel::Stable).unwrap();
        assert_eq!(stable.releases.len(), 1);

        let rc = list(&source, &v("1.0.0"), Channel::Rc).unwrap();
        assert_eq!(rc.releases.len(), 2);

        let alpha = list(&source, &v("1.0.0"), Channel::Alpha).unwrap();
        assert_eq!(alpha.releases.len(), 3);
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_skipped_rather_than_fatal() {
        // An unrelated tag in the repository must not hide every release.
        let source = Fake::with(&["v1.0.0", "nightly", "v1.1.0"]);
        let listing = list(&source, &v("1.0.0"), Channel::Stable).unwrap();
        assert_eq!(listing.releases.len(), 2);
    }

    #[test]
    fn asset_names_match_the_published_artifacts() {
        // These strings are shared with the release workflow; a change here
        // without a matching one there breaks installation for everyone.
        assert_eq!(asset_name("linux", "x86_64").unwrap(), "vump-linux-amd64");
        assert_eq!(asset_name("linux", "aarch64").unwrap(), "vump-linux-arm64");
        assert_eq!(
            asset_name("windows", "x86_64").unwrap(),
            "vump-windows-amd64.exe"
        );
        assert_eq!(
            asset_name("windows", "aarch64").unwrap(),
            "vump-windows-arm64.exe"
        );
        // Both macOS architectures resolve to the one notarized artifact.
        assert_eq!(
            asset_name("macos", "aarch64").unwrap(),
            "vump-darwin-universal"
        );
        assert_eq!(
            asset_name("macos", "x86_64").unwrap(),
            "vump-darwin-universal"
        );
    }

    #[test]
    fn tags_are_read_with_or_without_a_leading_v() {
        assert_eq!(parse_tag("v1.2.3").unwrap().version, v("1.2.3"));
        assert_eq!(parse_tag("1.2.3").unwrap().version, v("1.2.3"));
        // The tag is preserved as published, since it forms the download URL.
        assert_eq!(parse_tag("v1.2.3").unwrap().tag, "v1.2.3");
        assert!(parse_tag("nightly").is_none());
    }
}
