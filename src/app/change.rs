//! What a run will write, and the git side-effects that follow.
//!
//! Bumping and setting differ only in how they arrive at a target version.
//! Once that is decided, the work is identical: write the same version to every
//! tracked file, then optionally commit, tag and push. That shared part lives
//! here, so neither operation has to describe itself in the other's terms.

use std::path::{Path, PathBuf};

use semver::Version;
use thiserror::Error;

use crate::app::{AppError, FileVersion, resolve};
use crate::config::TagStyle;
use crate::config::{GitSettings, GitThrough, Project};
use crate::domain::TagPattern;
use crate::domain::TransitionError;
use crate::domain::version_file::Tracked;
use crate::ports::Annotation;
use crate::ports::{Availability, FileSystem, Vcs, VcsError};

/// A tracked file and the version it records today.
///
/// The version it will record is the change set's target, which is the same for
/// every file by construction: keeping the versions in step is the point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Path as declared in configuration.
    pub path: String,
    /// Version currently recorded.
    pub from: Version,
}

/// A tag to create, and how it will be written.
///
/// The annotation travels with the tag rather than beside it, so a run cannot
/// carry a message for a tag it will never create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagPlan {
    /// The tag name, rendered from the project's pattern.
    pub name: String,
    /// The message and signing choice, absent for a lightweight tag.
    pub annotation: Option<Annotation>,
}

impl TagPlan {
    /// How the tag is written, as it appears in configuration.
    #[must_use]
    pub fn style(&self) -> TagStyle {
        match &self.annotation {
            None => TagStyle::Lightweight,
            Some(annotation) if annotation.signed => TagStyle::Signed,
            Some(_) => TagStyle::Annotated,
        }
    }
}

/// The git side-effects a run will perform.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitPlan {
    /// Commit message, when a commit will be made.
    pub commit: Option<String>,
    /// The tag to create, when one will be.
    pub tag: Option<TagPlan>,
    /// Whether the commit and tag will be pushed.
    pub push: bool,
}

impl GitPlan {
    /// Whether the plan touches the repository at all.
    #[must_use]
    pub fn touches_repository(&self) -> bool {
        self.commit.is_some()
    }
}

/// Everything a run will do, decided before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    /// Project name, when the repository declares several.
    pub project: Option<String>,
    /// Version every tracked file will record.
    pub target: Version,
    /// Tracked files and what they record now, in declaration order.
    pub files: Vec<FileChange>,
    /// Git side-effects.
    pub git: GitPlan,
}

impl ChangeSet {
    /// The version every tracked file records, when they all agree.
    ///
    /// Derived rather than stored: the per-file versions are the truth, and a
    /// separate field could contradict them. Returns `None` when the files
    /// disagree, which a bump refuses outright and a set exists to repair.
    #[must_use]
    pub fn common_origin(&self) -> Option<&Version> {
        let mut versions = self.files.iter().map(|f| &f.from);
        let first = versions.next()?;
        versions.all(|other| other == first).then_some(first)
    }

    /// Whether any file's version would actually change.
    ///
    /// Writing the version the files already record changes nothing, and git
    /// refuses an empty commit, so callers check this before acting.
    #[must_use]
    pub fn changes_anything(&self) -> bool {
        self.files.iter().any(|f| f.from != self.target)
    }
}

/// What actually happened when a change set was applied.
///
/// Git steps are reported individually because a later step failing does not
/// undo an earlier one: a push that fails after a successful commit and tag
/// must be reported as exactly that, not as a total failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Files that were written.
    pub written: Vec<String>,
    /// Whether a commit was created.
    pub committed: bool,
    /// Whether a tag was created.
    pub tagged: bool,
    /// Whether the push succeeded.
    pub pushed: bool,
    /// Why the push failed, when it did.
    pub push_error: Option<String>,
}

/// How a release is named, and how far it is carried.
///
/// Naming travels with the step because the tag template is resolved per
/// project: two independently-versioned projects must not produce the same tag.
#[derive(Debug, Clone, Copy)]
pub struct GitPlanning<'a> {
    /// How far to carry the release.
    pub through: GitThrough,
    /// Commit message template.
    pub commit_message: &'a str,
    /// Tag template for the project being changed.
    pub tag: &'a TagPattern,
    /// How the tag object is written.
    pub tag_style: TagStyle,
    /// Message template for an annotated or signed tag.
    pub tag_message: &'a str,
}

/// Whether a run may act on a configuration nested inside another's.
///
/// Whether the branch a release is made from is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchCheck {
    /// Refuse a release from a branch the configuration does not list.
    Enforce,
    /// Proceed anyway: the caller passed `--any-branch`.
    Skipped,
}

/// A release being made from a branch the configuration does not list.
///
/// Returned rather than raised when the caller has waived the check, so the
/// waiver still says what was waived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffBranch {
    /// The branch `HEAD` is on, or `None` when it is detached.
    pub branch: Option<String>,
    /// Branches this maturity of release may be tagged from.
    pub allowed: Vec<String>,
    /// Whether the version being released is stable.
    pub stable: bool,
}

impl OffBranch {
    /// How this reads when it has to be reported rather than raised.
    ///
    /// The wording lives here so the guided menu, the warning a waived run
    /// prints, and the refusal itself cannot describe the same situation three
    /// different ways.
    #[must_use]
    pub fn describe(&self, check: BranchCheck) -> String {
        let key = if self.stable {
            "release_branches"
        } else {
            "prerelease_branches"
        };
        let where_it_is = match &self.branch {
            Some(branch) => format!("{branch} is not listed in {key}"),
            None => format!("HEAD is detached, and {key} is set"),
        };
        let way_out = match check {
            BranchCheck::Skipped => "--any-branch was passed, so this proceeds anyway",
            BranchCheck::Enforce => {
                "releases it governs are unavailable here, and --any-branch allows them"
            }
        };
        format!("warning: {where_it_is}; {way_out}.")
    }
}

/// Whether a configuration is acted on from below another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nesting {
    /// Refuse, naming the outer configuration and the way out.
    Refuse,
    /// Proceed: the caller passed `--allow-nested`.
    Allowed,
}

/// Why a change could not be planned or applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChangeError {
    /// A tracked file could not be read or interpreted.
    #[error("{0}")]
    App(#[from] AppError),

    /// The requested transition is not meaningful from the current version.
    #[error("{0}")]
    Transition(#[from] TransitionError),

    /// Tracked files record different versions.
    ///
    /// There is deliberately no flag to resolve this without a human: a source
    /// of truth contradicting itself is an exceptional state, not a parameter.
    /// `vump set` is the deliberate repair.
    #[error("tracked files disagree about the current version:\n{}", format_disagreement(.found))]
    OutOfSync {
        /// Each file and the version it records.
        found: Vec<(String, Version)>,
    },

    /// The working tree has uncommitted changes and the run would commit.
    #[error(
        "the working tree has uncommitted changes, so a version bump would sweep them into its commit:\n{}\n\ncommit or stash them first, or re-run with --through none",
        .changed.iter().map(|c| format!("  {c}")).collect::<Vec<_>>().join("\n")
    )]
    DirtyTree {
        /// Paths with uncommitted changes.
        changed: Vec<String>,
    },

    /// A configuration nested below another was acted on.
    ///
    /// Refused rather than warned: by the time a warning about a pushed tag is
    /// printed, the tag is on the remote, and a warning above a `check` verdict
    /// does not stop the verdict being believed.
    #[error(
        "{inner} sits below {outer}, so this acts on the nested project rather \
         than on the one above it: a bump would still commit and tag beyond what this \
         {file} describes, and a check would answer about the wrong project.\n\n\
         Several projects that version separately are declared as [[project]] entries \
         in one {file}, which is what lets them be addressed by name from anywhere. \
         Giving each its own {file} gives that up.\n\n\
         Pass --allow-nested to proceed anyway.",
        file = crate::config::FILE_NAME
    )]
    NestedConfig {
        /// The configuration in effect, in full.
        ///
        /// In full because a relative path needs a frame, and the reader cannot
        /// see which one was chosen: below two configurations, a path relative
        /// to the nearer one starts partway down a tree and reads as if it
        /// started at the top.
        inner: PathBuf,
        /// The configuration it shadows, in full.
        outer: PathBuf,
    },

    /// A release was made from a branch the configuration does not list.
    #[error(
        "{maturity} comes from {branches}, but HEAD is on {branch}.\n\n\
         A tag belongs to the branch it was made on. Delete that branch and the \
         tag is left pointing at a commit nothing reaches, which then has to be \
         removed by hand, locally and on the remote.\n\n\
         Switch branches, or pass --any-branch to release from here anyway.",
        maturity = if *stable { "a stable release" } else { "a pre-release" },
        branches = allowed.join(", ")
    )]
    OffBranch {
        /// The branch `HEAD` is on.
        branch: String,
        /// Branches this maturity of release may be tagged from.
        allowed: Vec<String>,
        /// Whether the version being released is stable.
        stable: bool,
    },

    /// A release was attempted with no branch checked out.
    #[error(
        "HEAD is detached, so a release tagged here would belong to no branch \
         at all and nothing would reach the commit it names.\n\n\
         Check out a branch, or pass --any-branch to release from here anyway."
    )]
    DetachedHead,

    /// A git operation failed.
    #[error("{0}")]
    Vcs(#[from] VcsError),
}

fn format_disagreement(found: &[(String, Version)]) -> String {
    let width = found.iter().map(|(p, _)| p.len()).max().unwrap_or_default();
    found
        .iter()
        .map(|(path, version)| format!("  {path:<width$}  {version}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Refuses to act on a configuration nested below another.
///
/// Every command touching a project is covered, reading included. The hazard
/// is not writing but using the wrong configuration at all: `check` answers
/// whether a version matches, and a nested project whose version happens to
/// coincide answers yes about the wrong project — a confident false pass from
/// the one command whose purpose is catching a version that lies. Refusing
/// only some commands would also warn about one layout sometimes and not
/// others, which reads as arbitrary and teaches nothing.
///
/// `init` is covered for the opposite reason: it is where the arrangement
/// every other command refuses would come into being.
///
/// Called once, where configuration becomes known and before any command runs,
/// so that a refusal costs no work and no later command can be written without
/// it.
///
/// # Errors
///
/// Returns [`ChangeError::NestedConfig`] when an outer configuration exists
/// and the caller has not acknowledged it.
pub fn check_nesting(
    fs: &dyn FileSystem,
    root: &Path,
    nesting: Nesting,
) -> Result<(), ChangeError> {
    if nesting == Nesting::Allowed {
        return Ok(());
    }
    let Some(outer) = crate::app::outer_config(fs, root) else {
        return Ok(());
    };

    Err(ChangeError::NestedConfig {
        inner: root.join(crate::config::FILE_NAME),
        outer,
    })
}

/// Refuses a release from a branch the configuration does not list.
///
/// Only a run that reaches a commit is covered. A run that writes files and
/// stops leaves nothing behind that could belong to the wrong branch, and
/// refusing it would block bumping a version in a pull request — which is
/// where a version is supposed to change.
///
/// That makes this narrower than [`check_nesting`], deliberately. Nesting
/// refuses reading too, because `check` can pass confidently about the wrong
/// project. Reading on a feature branch is not wrong in the same way: `status`
/// and `check` answer correctly wherever they are run.
///
/// Takes the target version rather than a composed plan, so an interactive run
/// can ask it as soon as the bump is chosen instead of after every remaining
/// question. One rule, asked from wherever the answer is first knowable.
///
/// # Errors
///
/// Returns [`ChangeError::OffBranch`] or [`ChangeError::DetachedHead`] when the
/// branch is not listed and the caller has not waived the check.
pub fn check_branch(
    vcs: &dyn Vcs,
    target: &Version,
    reaches_commit: bool,
    git: &GitSettings,
    check: BranchCheck,
) -> Result<Option<OffBranch>, ChangeError> {
    if !reaches_commit {
        return Ok(None);
    }
    let Some(allowed) = git.branches_for(target) else {
        return Ok(None);
    };

    let branch = vcs.current_branch()?;
    if branch
        .as_deref()
        .is_some_and(|on| allowed.iter().any(|listed| listed == on))
    {
        return Ok(None);
    }

    let off = OffBranch {
        branch,
        allowed: allowed.to_vec(),
        stable: target.pre.is_empty(),
    };

    match check {
        // Waived, not unnoticed: the caller still gets told what they waived.
        BranchCheck::Skipped => Ok(Some(off)),
        BranchCheck::Enforce => Err(match off.branch {
            Some(branch) => ChangeError::OffBranch {
                branch,
                allowed: off.allowed,
                stable: off.stable,
            },
            None => ChangeError::DetachedHead,
        }),
    }
}

/// How each candidate target stands against the release-branch policy.
///
/// One call rather than one per bump. The answer turns only on whether a
/// target is stable, so however many bumps a guided run offers there are at
/// most two distinct verdicts, and asking the repository twice is enough.
///
/// The same [`check_branch`] decides each one, waived, so what the menu shows
/// and what a run refuses can never disagree.
///
/// # Errors
///
/// Returns [`ChangeError::Vcs`] when the repository cannot be inspected.
pub fn offer(
    vcs: &dyn Vcs,
    targets: &[Version],
    reaches_commit: bool,
    git: &GitSettings,
    check: BranchCheck,
) -> Result<Vec<Availability>, ChangeError> {
    // Indexed by maturity: [stable, pre-release].
    let mut known: [Option<bool>; 2] = [None, None];
    let mut availability = Vec::with_capacity(targets.len());

    for target in targets {
        let slot = usize::from(!target.pre.is_empty());
        let off = if let Some(answer) = known[slot] {
            answer
        } else {
            let answer =
                check_branch(vcs, target, reaches_commit, git, BranchCheck::Skipped)?.is_some();
            known[slot] = Some(answer);
            answer
        };

        availability.push(match (off, check) {
            (false, _) => Availability::Free,
            (true, BranchCheck::Skipped) => Availability::Warned,
            (true, BranchCheck::Enforce) => Availability::Blocked,
        });
    }

    Ok(availability)
}

/// Builds the change set that writing `target` would produce.
#[must_use]
pub fn compose(
    project: &Project,
    target: Version,
    files: Vec<FileVersion>,
    planning: GitPlanning<'_>,
) -> ChangeSet {
    let git = GitPlan {
        commit: planning.through.commits().then(|| {
            GitSettings::render(planning.commit_message, project.name.as_deref(), &target)
        }),
        tag: planning.through.tags().then(|| TagPlan {
            name: planning.tag.render(&target),
            annotation: match planning.tag_style {
                TagStyle::Lightweight => None,
                TagStyle::Annotated | TagStyle::Signed => Some(Annotation {
                    message: GitSettings::render(
                        planning.tag_message,
                        project.name.as_deref(),
                        &target,
                    ),
                    signed: planning.tag_style == TagStyle::Signed,
                }),
            },
        }),
        push: planning.through.pushes(),
    };

    ChangeSet {
        project: project.name.clone(),
        target,
        files: files
            .into_iter()
            .map(|f| FileChange {
                path: f.path,
                from: f.version,
            })
            .collect(),
        git,
    }
}

/// Carries out a change set.
///
/// # Errors
///
/// Returns a [`ChangeError`] when the working tree is dirty and the run would
/// commit, when a file cannot be written, or when staging, committing or
/// tagging fails. A failed push is reported in the returned [`Outcome`] rather
/// than as an error, because the commit and tag it follows have already
/// succeeded.
pub fn apply(
    fs: &dyn FileSystem,
    vcs: &dyn Vcs,
    root: &Path,
    changes: &ChangeSet,
) -> Result<Outcome, ChangeError> {
    // Checked before anything is written: refusing after a partial write would
    // leave the tree in a state the user did not ask for.
    if changes.git.touches_repository() {
        let tree = vcs.status()?;
        if tree.is_dirty() {
            return Err(ChangeError::DirtyTree {
                changed: tree.changed,
            });
        }
    }

    // The same pairing the read side uses: a shared workspace lock records
    // every member, and this project's manifests say which entries are its own.
    let packages =
        crate::app::local_package_names(fs, root, changes.files.iter().map(|f| f.path.as_str()));

    let mut written = Vec::with_capacity(changes.files.len());
    for file in &changes.files {
        let absolute = resolve(root, &file.path);
        let file_name = absolute
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file.path.as_str());
        let tracked = Tracked::require(file_name).map_err(AppError::from)?;

        let contents = fs.read(&absolute).map_err(AppError::from)?;
        let updated = match tracked {
            Tracked::Manifest(format) => format.write(&file.path, &contents, &changes.target),
            Tracked::Lock(lock) => lock.write(&file.path, &contents, &changes.target, &packages),
        }
        .map_err(AppError::from)?;
        fs.write(&absolute, &updated).map_err(AppError::from)?;

        written.push(file.path.clone());
    }

    let mut outcome = Outcome {
        written,
        committed: false,
        tagged: false,
        pushed: false,
        push_error: None,
    };

    let Some(message) = changes.git.commit.as_deref() else {
        return Ok(outcome);
    };

    // Only the files vump manages are staged, so an unrelated change can never
    // ride along in a version-bump commit.
    let paths: Vec<String> = changes.files.iter().map(|f| f.path.clone()).collect();
    vcs.stage(&paths)?;
    vcs.commit(message)?;
    outcome.committed = true;

    if let Some(tag) = changes.git.tag.as_ref() {
        vcs.tag(&tag.name, tag.annotation.as_ref())?;
        outcome.tagged = true;
    }

    if changes.git.push {
        match vcs.push(changes.git.tag.as_ref().map(|t| t.name.as_str())) {
            Ok(()) => outcome.pushed = true,
            Err(e) => outcome.push_error = Some(e.to_string()),
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{MemoryFileSystem, MemoryVcs, VcsCall};
    use crate::config::DEFAULT_TAG_MESSAGE;
    use crate::config::DEFAULT_TAG_PATTERN;

    fn v(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn project() -> Project {
        Project {
            name: None,
            files: vec!["VERSION".to_owned()],
            tag_pattern: None,
        }
    }

    fn pattern() -> TagPattern {
        TagPattern::parse(DEFAULT_TAG_PATTERN).unwrap()
    }

    fn changeset(files: &[(&str, &str)], target: &str, through: GitThrough) -> ChangeSet {
        let settings = GitSettings::default();
        let pattern = pattern();
        compose(
            &project(),
            v(target),
            files
                .iter()
                .map(|(path, version)| FileVersion {
                    path: (*path).to_owned(),
                    version: v(version),
                })
                .collect(),
            GitPlanning {
                through,
                commit_message: &settings.commit_message,
                tag: &pattern,
                tag_style: TagStyle::default(),
                tag_message: DEFAULT_TAG_MESSAGE,
            },
        )
    }

    fn branch_policy(release: Option<&[&str]>, prerelease: Option<&[&str]>) -> GitSettings {
        let owned = |b: &[&str]| b.iter().map(|s| (*s).to_owned()).collect();
        GitSettings {
            release_branches: release.map(owned),
            prerelease_branches: prerelease.map(owned),
            ..GitSettings::default()
        }
    }

    #[test]
    fn a_stable_release_from_an_unlisted_branch_is_refused() {
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::Tag);

        let err = check_branch(
            &vcs,
            &changes.target,
            changes.git.touches_repository(),
            &branch_policy(Some(&["main"]), None),
            BranchCheck::Enforce,
        )
        .unwrap_err();

        assert!(matches!(err, ChangeError::OffBranch { .. }), "{err:?}");
        // Both halves of the fix have to be in the message: where releases come
        // from, and where the caller actually is.
        assert!(err.to_string().contains("feat/x"), "{err}");
        assert!(err.to_string().contains("main"), "{err}");
    }

    #[test]
    fn a_pre_release_answers_to_its_own_list() {
        // Locking stable releases to a branch is the common policy, and it must
        // not drag pre-releases along: sharing unfinished work would then mean
        // merging it first, which is what a pre-release exists to avoid.
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let policy = branch_policy(Some(&["main"]), None);

        let alpha = changeset(&[("VERSION", "1.2.3")], "1.3.0-alpha.0", GitThrough::Tag);
        assert_eq!(
            check_branch(
                &vcs,
                &alpha.target,
                alpha.git.touches_repository(),
                &policy,
                BranchCheck::Enforce
            )
            .unwrap(),
            None
        );

        let stable = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::Tag);
        assert!(
            check_branch(
                &vcs,
                &stable.target,
                stable.git.touches_repository(),
                &policy,
                BranchCheck::Enforce
            )
            .is_err()
        );
    }

    #[test]
    fn a_pre_release_can_be_constrained_on_its_own() {
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let alpha = changeset(&[("VERSION", "1.2.3")], "1.3.0-alpha.0", GitThrough::Tag);

        let err = check_branch(
            &vcs,
            &alpha.target,
            alpha.git.touches_repository(),
            &branch_policy(None, Some(&["develop"])),
            BranchCheck::Enforce,
        )
        .unwrap_err();

        assert!(err.to_string().contains("develop"), "{err}");
    }

    #[test]
    fn a_listed_branch_releases() {
        let vcs = MemoryVcs::new().on_branch("main");
        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::Tag);

        assert_eq!(
            check_branch(
                &vcs,
                &changes.target,
                changes.git.touches_repository(),
                &branch_policy(Some(&["main", "release"]), None),
                BranchCheck::Enforce
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_run_reaching_no_commit_is_not_governed() {
        // Writing files and stopping leaves nothing behind that could belong to
        // the wrong branch, and refusing it would block bumping a version in a
        // pull request.
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::None);

        assert_eq!(
            check_branch(
                &vcs,
                &changes.target,
                changes.git.touches_repository(),
                &branch_policy(Some(&["main"]), None),
                BranchCheck::Enforce
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_detached_head_is_refused_where_a_policy_exists() {
        let vcs = MemoryVcs::new().detached();
        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::Tag);

        let err = check_branch(
            &vcs,
            &changes.target,
            changes.git.touches_repository(),
            &branch_policy(Some(&["main"]), None),
            BranchCheck::Enforce,
        )
        .unwrap_err();

        assert!(matches!(err, ChangeError::DetachedHead), "{err:?}");
    }

    #[test]
    fn no_policy_governs_nothing() {
        // The whole check is opt-in, so a detached HEAD is only an obstacle to
        // someone who declared where releases come from.
        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::Tag);
        let settings = GitSettings::default();

        for vcs in [
            MemoryVcs::new().detached(),
            MemoryVcs::new().on_branch("feat/x"),
        ] {
            assert_eq!(
                check_branch(
                    &vcs,
                    &changes.target,
                    changes.git.touches_repository(),
                    &settings,
                    BranchCheck::Enforce
                )
                .unwrap(),
                None
            );
        }
    }

    #[test]
    fn a_menu_marks_what_the_policy_excludes() {
        // The list is the explanation: a bump that cannot be taken is shown
        // and marked, so its absence is never something to work out.
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let targets = [v("1.2.4"), v("1.3.0"), v("1.2.4-alpha.0")];

        let offered = offer(
            &vcs,
            &targets,
            true,
            &branch_policy(Some(&["main"]), None),
            BranchCheck::Enforce,
        )
        .unwrap();

        assert_eq!(
            offered,
            [
                Availability::Blocked,
                Availability::Blocked,
                Availability::Free
            ]
        );
    }

    #[test]
    fn waiving_the_check_marks_rather_than_clears() {
        // --any-branch restores the choice; it does not make it look safe.
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let targets = [v("1.2.4"), v("1.2.4-alpha.0")];

        let offered = offer(
            &vcs,
            &targets,
            true,
            &branch_policy(Some(&["main"]), None),
            BranchCheck::Skipped,
        )
        .unwrap();

        assert_eq!(offered, [Availability::Warned, Availability::Free]);
    }

    #[test]
    fn a_menu_asks_the_repository_once_per_maturity() {
        // However many bumps are offered, the answer turns only on whether the
        // target is stable, so the branch is looked up twice at most.
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let targets = [
            v("1.2.4"),
            v("1.3.0"),
            v("2.0.0"),
            v("1.2.4-alpha.0"),
            v("1.2.4-beta.0"),
        ];

        offer(
            &vcs,
            &targets,
            true,
            &branch_policy(Some(&["main"]), Some(&["develop"])),
            BranchCheck::Enforce,
        )
        .unwrap();

        let asked = vcs
            .calls()
            .iter()
            .filter(|c| matches!(c, VcsCall::Branch))
            .count();
        assert_eq!(asked, 2, "{:?}", vcs.calls());
    }

    #[test]
    fn a_run_that_reaches_no_commit_offers_everything() {
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let targets = [v("1.2.4"), v("1.2.4-alpha.0")];

        let offered = offer(
            &vcs,
            &targets,
            false,
            &branch_policy(Some(&["main"]), Some(&["main"])),
            BranchCheck::Enforce,
        )
        .unwrap();

        assert_eq!(offered, [Availability::Free, Availability::Free]);
    }

    #[test]
    fn waiving_the_check_still_reports_what_was_waived() {
        // --any-branch proceeds; it does not make the hazard go unmentioned.
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::Tag);

        let off = check_branch(
            &vcs,
            &changes.target,
            changes.git.touches_repository(),
            &branch_policy(Some(&["main"]), None),
            BranchCheck::Skipped,
        )
        .unwrap()
        .expect("a waived check still reports");

        assert_eq!(off.branch.as_deref(), Some("feat/x"));
        assert_eq!(off.allowed, ["main"]);
        assert!(off.stable);
    }

    /// Composes a tagging change set with `style` and `message`.
    fn tagged(style: TagStyle, message: &str, name: Option<&str>) -> ChangeSet {
        let pattern = TagPattern::parse(DEFAULT_TAG_PATTERN).unwrap();
        let settings = GitSettings::default();
        compose(
            &Project {
                name: name.map(ToOwned::to_owned),
                files: vec!["VERSION".to_owned()],
                tag_pattern: None,
            },
            v("1.2.4"),
            vec![FileVersion {
                path: "VERSION".to_owned(),
                version: v("1.2.3"),
            }],
            GitPlanning {
                through: GitThrough::Tag,
                commit_message: &settings.commit_message,
                tag: &pattern,
                tag_style: style,
                tag_message: message,
            },
        )
    }

    #[test]
    fn an_annotated_tag_carries_its_rendered_message() {
        let tag = tagged(TagStyle::Annotated, DEFAULT_TAG_MESSAGE, None)
            .git
            .tag
            .unwrap();

        assert_eq!(tag.name, "v1.2.4");
        assert_eq!(
            tag.annotation,
            Some(Annotation {
                message: "Release 1.2.4".to_owned(),
                signed: false,
            })
        );
    }

    #[test]
    fn a_lightweight_tag_carries_no_annotation() {
        let tag = tagged(TagStyle::Lightweight, DEFAULT_TAG_MESSAGE, None)
            .git
            .tag
            .unwrap();
        assert_eq!(tag.annotation, None);
        assert_eq!(tag.style(), TagStyle::Lightweight);
    }

    #[test]
    fn signing_is_a_property_of_the_annotation() {
        // There is no way to ask for a signature without a message to sign:
        // the two travel together or not at all.
        let tag = tagged(TagStyle::Signed, DEFAULT_TAG_MESSAGE, None)
            .git
            .tag
            .unwrap();
        let annotation = tag.annotation.expect("a signed tag is an annotated one");
        assert!(annotation.signed);
        assert_eq!(annotation.message, "Release 1.2.4");
    }

    #[test]
    fn a_tag_message_names_its_project() {
        let tag = tagged(TagStyle::Annotated, "{project} {new_version}", Some("api"))
            .git
            .tag
            .unwrap();
        assert_eq!(tag.annotation.unwrap().message, "api 1.2.4");
    }

    #[test]
    fn a_shared_version_is_derived_from_the_files() {
        let changes = changeset(
            &[("VERSION", "1.2.3"), ("Cargo.toml", "1.2.3")],
            "2.0.0",
            GitThrough::None,
        );
        assert_eq!(changes.common_origin(), Some(&v("1.2.3")));
    }

    #[test]
    fn files_that_disagree_have_no_shared_version() {
        // Deriving this rather than storing it means it cannot contradict the
        // per-file versions, which are the actual truth.
        let changes = changeset(
            &[("VERSION", "1.2.3"), ("Cargo.toml", "0.9.0")],
            "2.0.0",
            GitThrough::None,
        );
        assert_eq!(changes.common_origin(), None);
    }

    #[test]
    fn writing_the_recorded_version_changes_nothing() {
        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.3", GitThrough::None);
        assert!(!changes.changes_anything());
    }

    #[test]
    fn one_disagreeing_file_is_enough_to_be_a_change() {
        let changes = changeset(
            &[("VERSION", "1.2.3"), ("Cargo.toml", "0.9.0")],
            "1.2.3",
            GitThrough::None,
        );
        assert!(changes.changes_anything());
    }

    #[test]
    fn applying_writes_every_file() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.2.3\n")
            .with_file("/repo/Cargo.toml", "[package]\nversion = \"1.2.3\"\n");
        let vcs = MemoryVcs::new();

        let changes = changeset(
            &[("VERSION", "1.2.3"), ("Cargo.toml", "1.2.3")],
            "1.3.0",
            GitThrough::None,
        );
        let outcome = apply(&fs, &vcs, Path::new("/repo"), &changes).unwrap();

        assert_eq!(outcome.written.len(), 2);
        assert_eq!(fs.get("/repo/VERSION").as_deref(), Some("1.3.0\n"));
        assert!(fs.get("/repo/Cargo.toml").unwrap().contains("1.3.0"));
        // With no git intent, the repository is untouched.
        assert_eq!(vcs.calls(), Vec::new());
    }

    #[test]
    fn commit_and_tag_use_the_configured_templates() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");
        let vcs = MemoryVcs::new();

        let changes = changeset(&[("VERSION", "1.2.3")], "1.2.4", GitThrough::Tag);
        let outcome = apply(&fs, &vcs, Path::new("/repo"), &changes).unwrap();

        assert!(outcome.committed && outcome.tagged && !outcome.pushed);
        assert_eq!(
            vcs.calls(),
            [
                VcsCall::Stage(vec!["VERSION".to_owned()]),
                VcsCall::Commit("chore: bump version to v1.2.4".to_owned()),
                VcsCall::Tag(
                    "v1.2.4".to_owned(),
                    // The default style: annotated, with the default message.
                    Some(Annotation {
                        message: "Release 1.2.4".to_owned(),
                        signed: false,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn a_dirty_tree_stops_the_run_before_anything_is_written() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.0.0\n");
        let vcs = MemoryVcs::new().with_changes(&["src/main.rs"]);

        let changes = changeset(&[("VERSION", "1.0.0")], "1.0.1", GitThrough::Commit);

        let err = apply(&fs, &vcs, Path::new("/repo"), &changes).unwrap_err();
        assert!(matches!(err, ChangeError::DirtyTree { .. }));
        // The refusal must leave the file exactly as it was.
        assert_eq!(fs.get("/repo/VERSION").as_deref(), Some("1.0.0\n"));
    }

    #[test]
    fn a_dirty_tree_is_irrelevant_when_no_commit_is_planned() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.0.0\n");
        let vcs = MemoryVcs::new().with_changes(&["src/main.rs"]);

        let changes = changeset(&[("VERSION", "1.0.0")], "1.0.1", GitThrough::None);
        assert!(apply(&fs, &vcs, Path::new("/repo"), &changes).is_ok());
    }

    #[test]
    fn a_failed_push_reports_what_already_succeeded() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.0.0\n");
        let vcs = MemoryVcs::new().failing("push", "no upstream configured");

        let changes = changeset(&[("VERSION", "1.0.0")], "1.0.1", GitThrough::Push);

        // The commit and tag really happened, so this is not a failed run.
        let outcome = apply(&fs, &vcs, Path::new("/repo"), &changes).unwrap();
        assert!(outcome.committed);
        assert!(outcome.tagged);
        assert!(!outcome.pushed);
        assert!(outcome.push_error.is_some());
    }

    #[test]
    fn only_tracked_files_are_staged() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.0.0\n")
            .with_file("/repo/unrelated.txt", "untouched");
        let vcs = MemoryVcs::new();

        let changes = changeset(&[("VERSION", "1.0.0")], "1.0.1", GitThrough::Commit);
        apply(&fs, &vcs, Path::new("/repo"), &changes).unwrap();

        assert_eq!(vcs.calls()[0], VcsCall::Stage(vec!["VERSION".to_owned()]));
    }

    #[test]
    fn each_step_performs_every_step_before_it() {
        // The ladder is what makes a combination like "tag without a commit"
        // impossible to construct, so the inclusion has to hold at every rung.
        assert!(!GitThrough::None.commits());
        assert!(GitThrough::Commit.commits());
        assert!(
            GitThrough::Tag.commits(),
            "a tag needs a commit to point at"
        );
        assert!(
            GitThrough::Push.commits(),
            "there is nothing to push without a commit"
        );
        assert!(GitThrough::Push.tags(), "the pushed ref is the tag");

        assert!(!GitThrough::Commit.tags());
        assert!(!GitThrough::Tag.pushes());
    }

    #[test]
    fn the_first_step_touches_nothing() {
        let changes = changeset(&[("VERSION", "1.0.0")], "1.0.1", GitThrough::None);
        assert!(!changes.git.touches_repository());
    }
}
