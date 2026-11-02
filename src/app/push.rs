//! Pushing what a bump already created.
//!
//! A bump stopping at `tag` leaves a commit and a tag behind, and finishing by
//! hand with `git push --follow-tags` pushes every annotated tag reachable
//! from `HEAD` — in a repository tagging several projects independently, that
//! publishes whichever other project's tag happened to be sitting unpushed.
//! Naming the one ref is the whole point of this command.
//!
//! It stores nothing. The tag is rendered from the version the tracked files
//! already record and the project's own pattern, which is the same derivation
//! the bump used to create it.

use std::path::Path;

use semver::Version;
use thiserror::Error;

use crate::app::{self, AppError};
use crate::config::{Config, ConfigError, Project};
use crate::ports::{FileSystem, Vcs, VcsError};

/// What a push did, or would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    /// The version the tracked files record.
    pub version: Version,
    /// The tag pushed alongside the branch.
    pub tag: String,
}

/// Why a push could not be performed.
#[derive(Debug, Error)]
pub enum PushError {
    /// The configuration could not be read or the project not selected.
    #[error("{0}")]
    Config(#[from] ConfigError),

    /// The tracked files could not be read.
    #[error("{0}")]
    App(#[from] AppError),

    /// Tracked files record different versions, so no single tag follows.
    #[error("tracked files disagree about the current version:\n{}", .found.iter().map(|(p, v)| format!("  {p}  {v}")).collect::<Vec<_>>().join("\n"))]
    OutOfSync {
        /// Each file and the version it records.
        found: Vec<(String, Version)>,
    },

    /// The tag the files imply does not exist, so there is nothing to push.
    #[error(
        "no tag {tag} exists for version {version}; run a bump that reaches \
         `tag` first, or push the branch with git if that is all you meant"
    )]
    NoTag {
        /// The tag that was looked for.
        tag: String,
        /// The version the tracked files record.
        version: Version,
    },

    /// Git refused the push.
    #[error("{0}")]
    Vcs(#[from] VcsError),
}

/// Renders the tag the tracked files imply, without pushing anything.
///
/// # Errors
///
/// Returns a [`PushError`] when the files cannot be read, disagree, or the tag
/// they imply does not exist locally.
pub fn plan(
    fs: &dyn FileSystem,
    vcs: &dyn Vcs,
    root: &Path,
    config: &Config,
    project: &Project,
) -> Result<Pushed, PushError> {
    let files = app::read_project_versions(fs, root, project)?;
    let version = app::agreed_version(&files).map_err(|found| PushError::OutOfSync { found })?;
    let tag = config.tag_pattern_for(project)?.render(&version);

    // Refusing here rather than pushing the branch alone: a caller asking for
    // this command wants the tag moved, and a push that silently carried only
    // half of that would look like it had done the job.
    if !vcs.has_tag(&tag)? {
        return Err(PushError::NoTag { tag, version });
    }

    Ok(Pushed { version, tag })
}

/// Pushes `HEAD` and the one tag the tracked files imply.
///
/// # Errors
///
/// Returns a [`PushError`] when the plan cannot be formed or the push fails.
pub fn push(
    fs: &dyn FileSystem,
    vcs: &dyn Vcs,
    root: &Path,
    config: &Config,
    project: &Project,
) -> Result<Pushed, PushError> {
    let pushed = plan(fs, vcs, root, config, project)?;
    vcs.push(Some(&pushed.tag))?;
    Ok(pushed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{MemoryFileSystem, MemoryVcs, VcsCall};

    fn config(pattern: &str) -> Config {
        crate::config::Config::parse(
            Path::new("/repo/vump.toml"),
            &format!("files = [\"VERSION\"]\n\n[git]\ntag_pattern = \"{pattern}\"\n"),
        )
        .expect("fixture configuration parses")
    }

    fn files(version: &str) -> MemoryFileSystem {
        let fs = MemoryFileSystem::new();
        fs.insert("/repo/VERSION", format!("{version}\n"));
        fs
    }

    #[test]
    fn pushes_only_the_tag_the_files_imply() {
        // The point of the command: `git push --follow-tags` would also carry
        // the second tag, which belongs to nothing this project released.
        let fs = files("1.0.1");
        let vcs = MemoryVcs::new().with_tags(&["api-v1.0.1", "other-project-v9.9.9"]);
        let config = config("api-v{new_version}");
        let project = config.select(None).expect("single project");

        let pushed = push(&fs, &vcs, Path::new("/repo"), &config, project).expect("push succeeds");

        assert_eq!(pushed.tag, "api-v1.0.1");
        assert!(
            vcs.calls()
                .contains(&VcsCall::Push(Some("api-v1.0.1".to_owned()))),
            "{:?}",
            vcs.calls()
        );
        assert!(
            !vcs.calls()
                .iter()
                .any(|c| matches!(c, VcsCall::Push(Some(t)) if t.contains("other-project"))),
            "{:?}",
            vcs.calls()
        );
    }

    #[test]
    fn a_tag_that_was_never_made_is_refused_before_pushing() {
        // Pushing the branch alone would look like the job was done.
        let fs = files("1.0.1");
        let vcs = MemoryVcs::new();
        let config = config("v{new_version}");
        let project = config.select(None).expect("single project");

        let err = push(&fs, &vcs, Path::new("/repo"), &config, project).unwrap_err();
        assert!(matches!(err, PushError::NoTag { .. }), "{err:?}");
        assert!(
            !vcs.calls().iter().any(|c| matches!(c, VcsCall::Push(_))),
            "{:?}",
            vcs.calls()
        );
    }

    #[test]
    fn planning_pushes_nothing() {
        let fs = files("1.0.1");
        let vcs = MemoryVcs::new().with_tags(&["v1.0.1"]);
        let config = config("v{new_version}");
        let project = config.select(None).expect("single project");

        let planned = plan(&fs, &vcs, Path::new("/repo"), &config, project).expect("plan succeeds");

        assert_eq!(planned.tag, "v1.0.1");
        assert!(
            !vcs.calls().iter().any(|c| matches!(c, VcsCall::Push(_))),
            "{:?}",
            vcs.calls()
        );
    }

    #[test]
    fn files_that_disagree_imply_no_single_tag() {
        let fs = MemoryFileSystem::new();
        fs.insert("/repo/VERSION", "1.0.1\n");
        fs.insert("/repo/sub/VERSION", "2.0.0\n");
        let config = crate::config::Config::parse(
            Path::new("/repo/vump.toml"),
            "files = [\"VERSION\", \"sub/VERSION\"]\n",
        )
        .expect("fixture parses");
        let project = config.select(None).expect("single project");
        let vcs = MemoryVcs::new().with_tags(&["v1.0.1", "v2.0.0"]);

        let err = push(&fs, &vcs, Path::new("/repo"), &config, project).unwrap_err();
        assert!(matches!(err, PushError::OutOfSync { .. }), "{err:?}");
    }
}
