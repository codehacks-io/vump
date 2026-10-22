//! Traits describing what use cases need from the outside world.
//!
//! Use cases depend on these traits, never on their implementations. That is
//! what allows the interesting behavior to be exercised in tests without a
//! filesystem, a git repository, or a terminal.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::GitThrough;

/// Filesystem access required by use cases.
pub trait FileSystem {
    /// Reads a file as UTF-8 text.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::NotFound`] when the file does not exist, and
    /// [`FsError::Io`] for any other failure, including invalid UTF-8.
    fn read(&self, path: &Path) -> Result<String, FsError>;

    /// Writes UTF-8 text, replacing any existing contents.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Io`] when the write cannot be completed.
    fn write(&self, path: &Path, contents: &str) -> Result<(), FsError>;

    /// Whether a regular file exists at `path`.
    fn is_file(&self, path: &Path) -> bool;

    /// Lists the entries of a directory, in unspecified order.
    ///
    /// # Errors
    ///
    /// Returns [`FsError`] when the directory cannot be listed.
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>, FsError>;
}

/// One entry within a directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DirEntry {
    /// The entry's own name, without any leading path.
    pub name: String,
    /// Whether the entry is itself a directory.
    pub is_dir: bool,
}

/// How a bump stands against the repository's release-branch policy.
///
/// Three states rather than two flags: a bump cannot be both blocked and
/// merely warned about, and a pair of booleans would let that be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Offered without qualification.
    Free,
    /// Offered, but it releases from a branch the policy excludes.
    ///
    /// Reached by waiving the check rather than by satisfying it, so the risk
    /// is marked instead of hidden.
    Warned,
    /// Shown, but refused if chosen: the policy excludes it.
    Blocked,
}

/// One bump a guided run offers, and whether it may be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionChoice {
    /// What the bump is called.
    pub label: String,
    /// The version it would produce.
    pub result: String,
    /// Whether the release-branch policy permits it.
    pub availability: Availability,
}

/// Questions a run may need to put to a person.
///
/// Only the interactive entry point uses this. Naming a subcommand selects an
/// implementation that cannot ask anything, which is how the non-interactive
/// guarantee is enforced by construction rather than by discipline.
pub trait Interaction {
    /// Asks which project to operate on.
    ///
    /// # Errors
    ///
    /// Returns [`InteractionError`] when the question cannot be asked or is
    /// declined.
    fn choose_project(&self, names: &[String]) -> Result<String, InteractionError>;

    /// Asks which version to treat as current when tracked files disagree.
    ///
    /// # Errors
    ///
    /// Returns [`InteractionError`] when the question cannot be asked or is
    /// declined.
    fn choose_base(&self, candidates: &[(String, String)]) -> Result<String, InteractionError>;

    /// Asks which transition to apply, given those valid from here.
    ///
    /// `options` contains every transition that would succeed as a version
    /// change. One the release-branch policy excludes is shown rather than
    /// dropped, so the reason it cannot be taken is visible instead of being
    /// inferred from an absence, and choosing it is refused rather than
    /// accepted.
    ///
    /// Returns an index into `options`, never one that is
    /// [`Availability::Blocked`].
    ///
    /// # Errors
    ///
    /// Returns [`InteractionError`] when the question cannot be asked or is
    /// declined.
    fn choose_transition(
        &self,
        current: &str,
        options: &[TransitionChoice],
    ) -> Result<usize, InteractionError>;

    /// Asks how far to carry the release.
    ///
    /// Asked only when configuration has not already decided. It is one
    /// question rather than three because the steps are cumulative: answering
    /// "tag" has already answered "commit".
    ///
    /// # Errors
    ///
    /// Returns [`InteractionError`] when the question cannot be asked or is
    /// declined.
    fn choose_git(&self) -> Result<GitThrough, InteractionError>;

    /// Reports something the person should see before the next question.
    ///
    /// Not a result and not a failure: a hazard that changes what the questions
    /// after it mean. It goes through the port rather than being printed
    /// directly so that a run driven from memory can see it too.
    fn notice(&self, message: &str);

    /// Asks for final approval of a rendered summary.
    ///
    /// # Errors
    ///
    /// Returns [`InteractionError`] when the question cannot be asked or is
    /// declined.
    fn confirm(&self, summary: &str) -> Result<bool, InteractionError>;
}

/// Why a question could not be answered.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InteractionError {
    /// The person declined, by cancelling the prompt.
    #[error("cancelled")]
    Cancelled,

    /// There is nobody to ask.
    #[error("cannot prompt: {detail}")]
    Unavailable {
        /// Why prompting is impossible, for example no terminal is attached.
        detail: String,
    },
}

/// The message carried by a tag that is more than a bare pointer.
///
/// A signed tag is always an annotated one, so signing is a property of the
/// annotation rather than an independent setting: there is no way to ask for a
/// signature without a message to sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// Message recorded on the tag object.
    pub message: String,
    /// Whether to sign the tag object.
    pub signed: bool,
}

/// Version control operations required by use cases.
///
/// Implementations act on one repository, fixed at construction, so that no
/// caller has to thread a working directory through every call.
pub trait Vcs {
    /// Reports whether the working tree has uncommitted changes.
    ///
    /// # Errors
    ///
    /// Returns [`VcsError`] when the repository cannot be inspected.
    fn status(&self) -> Result<WorkingTree, VcsError>;

    /// The branch `HEAD` points at, or `None` when `HEAD` is detached.
    ///
    /// The absence is genuine rather than a failure: a detached `HEAD` is a
    /// state a repository is legitimately in, and what it means for the
    /// operation at hand is the caller's decision, not this port's.
    ///
    /// # Errors
    ///
    /// Returns [`VcsError`] when the repository cannot be inspected.
    fn current_branch(&self) -> Result<Option<String>, VcsError>;

    /// Stages the given paths, which are relative to the repository root.
    ///
    /// # Errors
    ///
    /// Returns [`VcsError`] when staging fails.
    fn stage(&self, paths: &[String]) -> Result<(), VcsError>;

    /// Commits the staged changes.
    ///
    /// # Errors
    ///
    /// Returns [`VcsError`] when the commit fails.
    fn commit(&self, message: &str) -> Result<(), VcsError>;

    /// Creates a tag at `HEAD`.
    ///
    /// # Errors
    ///
    /// Returns [`VcsError`] when the tag cannot be created, including when one
    /// of that name already exists.
    fn tag(&self, name: &str, annotation: Option<&Annotation>) -> Result<(), VcsError>;

    /// Whether a tag of this name exists locally.
    ///
    /// # Errors
    ///
    /// Returns [`VcsError`] when the repository cannot be inspected.
    fn has_tag(&self, name: &str) -> Result<bool, VcsError>;

    /// Pushes the current branch, and `tag` when one is given.
    ///
    /// # Errors
    ///
    /// Returns [`VcsError`] when either push fails.
    fn push(&self, tag: Option<&str>) -> Result<(), VcsError>;
}

/// The state of a working tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkingTree {
    /// Paths with uncommitted changes, as reported by the VCS.
    pub changed: Vec<String>,
}

impl WorkingTree {
    /// Whether anything is uncommitted.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        !self.changed.is_empty()
    }
}

/// A version control operation that could not be completed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VcsError {
    /// The command could not be run at all.
    #[error("cannot run git: {detail}")]
    Unavailable {
        /// Underlying error detail.
        detail: String,
    },

    /// The command ran and failed.
    #[error("git {operation} failed: {detail}")]
    Failed {
        /// The operation being attempted, for example `commit`.
        operation: String,
        /// Output produced by git.
        detail: String,
    },
}

/// A filesystem operation that could not be completed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FsError {
    /// The path does not exist.
    #[error("{path} does not exist")]
    NotFound {
        /// The path that was looked for.
        path: PathBuf,
    },

    /// Any other failure, including permission errors and invalid UTF-8.
    #[error("cannot access {path}: {detail}")]
    Io {
        /// The path being accessed.
        path: PathBuf,
        /// Underlying error detail.
        detail: String,
    },
}
