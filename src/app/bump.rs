//! Advancing a version by a semver transition.
//!
//! Planning is a pure decision: it reads what is recorded, works out the new
//! version, and describes the change that would follow — without writing
//! anything. Carrying it out is [`crate::app::change::apply`], shared with
//! [`crate::app::set`].
//!
//! That split is what lets `--dry-run`, the JSON renderer and the human
//! renderer all describe exactly what a real run would do: all three read the
//! same change set the real run executes.

use std::path::Path;

use semver::Version;

use crate::app::change::{ChangeError, ChangeSet, GitPlanning, compose};
use crate::app::read_project_versions;
use crate::config::Project;
use crate::domain::{Transition, apply as transition_apply};

pub use crate::domain::bump::valid_transitions as valid_transitions_for;
use crate::ports::FileSystem;

/// A bump, and the change set carrying it out.
///
/// A bump always knows the version it moves from — either every tracked file
/// agreed on it, or a person resolved the disagreement and chose one — so it is
/// carried plainly rather than as something that may be absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BumpPlan {
    /// The version bumped from.
    pub from: Version,
    /// What will be written, and the git side-effects that follow.
    pub changes: ChangeSet,
}

/// Decides what a bump would change, without writing anything.
///
/// # Errors
///
/// Returns a [`ChangeError`] when files cannot be read, disagree about the
/// current version, or the transition is not meaningful.
pub fn plan(
    fs: &dyn FileSystem,
    root: &Path,
    project: &Project,
    transition: Transition,
    planning: GitPlanning<'_>,
) -> Result<BumpPlan, ChangeError> {
    plan_from(fs, root, project, None, transition, planning)
}

/// Decides what a bump would change, treating `base` as the current version.
///
/// Passing a `base` skips the requirement that tracked files already agree,
/// which is how an interactive run proceeds from a disagreement the person has
/// just been shown and resolved. Passing `None` requires agreement, as [`plan`]
/// does.
///
/// # Errors
///
/// Returns a [`ChangeError`] when files cannot be read, disagree with no `base`
/// given, or the transition is not meaningful.
pub fn plan_from(
    fs: &dyn FileSystem,
    root: &Path,
    project: &Project,
    base: Option<Version>,
    transition: Transition,
    planning: GitPlanning<'_>,
) -> Result<BumpPlan, ChangeError> {
    let files = read_project_versions(fs, root, project)?;

    let from = if let Some(base) = base {
        base
    } else {
        crate::app::agreed_version(&files).map_err(|found| ChangeError::OutOfSync { found })?
    };

    let target = transition_apply(&from, transition)?;

    Ok(BumpPlan {
        from,
        changes: compose(project, target, files, planning),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::MemoryFileSystem;
    use crate::app::change::GitPlan;
    use crate::app::change::TagPlan;
    use crate::config::{DEFAULT_TAG_PATTERN, GitSettings, GitThrough};
    use crate::domain::{StableBump, TagPattern};
    use crate::ports::Annotation;

    fn project(files: &[&str]) -> Project {
        Project {
            name: None,
            files: files.iter().map(|s| (*s).to_owned()).collect(),
            tag_pattern: None,
        }
    }

    fn v(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn root() -> &'static Path {
        Path::new("/repo")
    }

    fn settings() -> GitSettings {
        GitSettings::default()
    }

    fn default_pattern() -> TagPattern {
        TagPattern::parse(DEFAULT_TAG_PATTERN).expect("default pattern is valid")
    }

    fn planning<'a>(
        through: GitThrough,
        settings: &'a GitSettings,
        tag: &'a TagPattern,
    ) -> GitPlanning<'a> {
        GitPlanning {
            through,
            commit_message: &settings.commit_message,
            tag,
            tag_style: settings.tag_style,
            tag_message: &settings.tag_message,
        }
    }

    #[test]
    fn plans_a_patch_across_every_file() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.2.3\n")
            .with_file("/repo/ui/package.json", r#"{"version":"1.2.3"}"#);

        let plan = plan(
            &fs,
            root(),
            &project(&["VERSION", "ui/package.json"]),
            Transition::Stable(StableBump::Patch),
            planning(GitThrough::None, &settings(), &default_pattern()),
        )
        .unwrap();

        assert_eq!(plan.from, v("1.2.3"));
        assert_eq!(plan.changes.target, v("1.2.4"));
        assert_eq!(plan.changes.files.len(), 2);
        assert!(!plan.changes.git.touches_repository());
    }

    #[test]
    fn planning_writes_nothing() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");

        plan(
            &fs,
            root(),
            &project(&["VERSION"]),
            Transition::Stable(StableBump::Major),
            planning(GitThrough::None, &settings(), &default_pattern()),
        )
        .unwrap();

        assert_eq!(fs.get("/repo/VERSION").as_deref(), Some("1.2.3\n"));
    }

    #[test]
    fn disagreeing_files_are_refused() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.2.3\n")
            .with_file("/repo/Cargo.toml", "[package]\nversion = \"1.0.0\"\n");

        let err = plan(
            &fs,
            root(),
            &project(&["VERSION", "Cargo.toml"]),
            Transition::Stable(StableBump::Patch),
            planning(GitThrough::None, &settings(), &default_pattern()),
        )
        .unwrap_err();

        let ChangeError::OutOfSync { found } = err else {
            panic!("expected OutOfSync, got {err:?}");
        };
        // Every file is listed, not just the odd one out: which is "wrong" is
        // the user's call.
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn a_resolved_base_proceeds_despite_disagreement() {
        // What an interactive run does once the person has chosen.
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.2.3\n")
            .with_file("/repo/Cargo.toml", "[package]\nversion = \"1.0.0\"\n");

        let plan = plan_from(
            &fs,
            root(),
            &project(&["VERSION", "Cargo.toml"]),
            Some(v("1.2.3")),
            Transition::Stable(StableBump::Patch),
            planning(GitThrough::None, &settings(), &default_pattern()),
        )
        .unwrap();

        assert_eq!(plan.from, v("1.2.3"));
        assert_eq!(plan.changes.target, v("1.2.4"));
    }

    #[test]
    fn an_invalid_transition_is_refused_at_planning_time() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3-rc.1\n");

        let err = plan(
            &fs,
            root(),
            &project(&["VERSION"]),
            Transition::Stable(StableBump::Patch),
            planning(GitThrough::None, &settings(), &default_pattern()),
        )
        .unwrap_err();

        assert!(matches!(err, ChangeError::Transition(_)));
    }

    #[test]
    fn the_tag_names_the_version_being_written() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");

        let plan = plan(
            &fs,
            root(),
            &project(&["VERSION"]),
            Transition::Stable(StableBump::Minor),
            planning(GitThrough::Tag, &settings(), &default_pattern()),
        )
        .unwrap();

        assert_eq!(
            plan.changes.git,
            GitPlan {
                commit: Some("chore: bump version to v1.3.0".to_owned()),
                tag: Some(TagPlan {
                    name: "v1.3.0".to_owned(),
                    annotation: Some(Annotation {
                        message: "Release 1.3.0".to_owned(),
                        signed: false,
                    }),
                }),
                push: false,
            }
        );
    }
}
