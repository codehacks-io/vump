//! The guided run: a bump whose decisions are asked for rather than passed in.
//!
//! It lives here rather than in the CLI because it is the operation carrying
//! the most decisions, and a decision that cannot be driven from memory cannot
//! be tested. Every question and every side-effect goes through a port, so the
//! whole run is exercisable without a terminal or a repository.
//!
//! Nothing here formats output. The confirmation text is rendered by the
//! caller and passed in, so presentation stays where the rest of it lives.

use std::path::Path;

use semver::Version;
use thiserror::Error;

use crate::app::bump::BumpPlan;
use crate::app::change::{BranchCheck, ChangeError, GitPlanning, Outcome};
use crate::app::{self, AppError, FileVersion};
use crate::config::{Config, ConfigError};
use crate::domain::{Transition, TransitionError};
use crate::ports::{
    Availability, FileSystem, Interaction, InteractionError, TransitionChoice, Vcs,
};

/// A guided run that went through.
#[derive(Debug)]
pub struct Application {
    /// What the run planned to do.
    pub plan: BumpPlan,
    /// What the repository did about it.
    pub outcome: Outcome,
}

/// What a guided run settled on.
///
/// The applied case is boxed because declining carries nothing: without it,
/// every caller would pay for the larger variant to say "nothing happened".
#[derive(Debug)]
pub enum Guided {
    /// Declined at the confirmation. Nothing was written.
    Declined,
    /// Approved, and applied.
    Applied(Box<Application>),
}

/// What a guided run is told before it starts asking.
#[derive(Debug, Clone, Copy)]
pub struct Guidance<'a> {
    /// Project named by the caller, when one was.
    pub project: Option<&'a str>,
    /// Whether the release-branch policy is enforced or waived.
    pub branch: BranchCheck,
    /// Renders a plan for the final confirmation.
    ///
    /// Passed in rather than called directly: which words a person sees is the
    /// caller's business, and a test needs neither.
    pub summary: fn(&BumpPlan) -> String,
    /// Names a bump for the menu, in the caller's own vocabulary.
    pub label: fn(Transition) -> String,
}

/// Why a guided run could not complete.
#[derive(Debug, Error)]
pub enum GuidedError {
    /// Configuration does not describe what was asked for.
    #[error("{0}")]
    Config(#[from] ConfigError),

    /// A tracked file could not be read or interpreted.
    #[error("{0}")]
    App(#[from] AppError),

    /// The change could not be planned or applied.
    #[error("{0}")]
    Change(#[from] ChangeError),

    /// No version transition is possible from here.
    #[error("{0}")]
    Transition(#[from] TransitionError),

    /// A question could not be asked, or was declined.
    #[error("{0}")]
    Interaction(#[from] InteractionError),
}

/// Guides a bump, asking only what has not already been decided.
///
/// Configuration is consulted first: a repository that declares its git
/// settings is never asked about them again. That leaves two questions in the
/// common case — which bump, and whether to proceed.
///
/// # Errors
///
/// Returns [`GuidedError`] when configuration, the files, the repository or the
/// person answering prevents the run from completing.
pub fn run(
    fs: &dyn FileSystem,
    vcs: &dyn Vcs,
    ask: &dyn Interaction,
    root: &Path,
    config: &Config,
    guidance: Guidance<'_>,
) -> Result<Guided, GuidedError> {
    // Which project.
    let project = match guidance.project {
        Some(name) => config.select(Some(name))?.clone(),
        None if config.projects.len() == 1 => config.projects[0].clone(),
        None => {
            let names: Vec<String> = config
                .projects
                .iter()
                .filter_map(|p| p.name.clone())
                .collect();
            let chosen = ask.choose_project(&names)?;
            config.select(Some(&chosen))?.clone()
        }
    };

    // What the current version is, resolving a disagreement if there is one.
    let files = app::read_project_versions(fs, root, &project)?;
    let base = resolve_base(ask, &files)?;

    // Which transition, offering only those that would succeed as versions.
    let offered = app::bump::valid_transitions_for(&base)?;
    let targets: Vec<Version> = offered.iter().map(|(_, next)| next.clone()).collect();

    // What the branch policy makes of each, settled before the question is
    // asked. A run that would be refused must not collect answers first, and a
    // bump that cannot be taken should say so in the list rather than be
    // discovered afterwards.
    //
    // With `through` unset the run counts as reaching git, because that is what
    // it does unless the next question says otherwise.
    let reaches_commit = config
        .git
        .through
        .is_none_or(crate::config::GitThrough::commits);
    let availability =
        app::change::offer(vcs, &targets, reaches_commit, &config.git, guidance.branch)?;

    // Whichever bump the policy has something to say about explains it for all
    // of them: the branch and the list are the same either way.
    if let Some(qualified) = availability.iter().position(|a| *a != Availability::Free) {
        if let Some(off) = app::change::check_branch(
            vcs,
            &targets[qualified],
            reaches_commit,
            &config.git,
            BranchCheck::Skipped,
        )? {
            ask.notice(&off.describe(guidance.branch));
        }
    }

    // Nothing selectable is a refusal, not a menu. Raised through the same
    // check, so the wording is the one every other refusal uses.
    if availability.iter().all(|a| *a == Availability::Blocked) {
        app::change::check_branch(
            vcs,
            &targets[0],
            reaches_commit,
            &config.git,
            BranchCheck::Enforce,
        )?;
    }

    let choices: Vec<TransitionChoice> = offered
        .iter()
        .zip(&availability)
        .map(|((transition, next), availability)| TransitionChoice {
            label: (guidance.label)(*transition),
            result: next.to_string(),
            availability: *availability,
        })
        .collect();

    let index = ask.choose_transition(&base.to_string(), &choices)?;
    let (transition, _) = offered.get(index).ok_or(InteractionError::Cancelled)?;

    // The questioner is asked not to return a blocked bump, and not trusted to
    // have obeyed. Refused through the same check, so an implementation that
    // ignores the marking is stopped by the rule rather than by good manners.
    if availability[index] == Availability::Blocked {
        app::change::check_branch(
            vcs,
            &targets[index],
            reaches_commit,
            &config.git,
            BranchCheck::Enforce,
        )?;
    }

    // How far to carry the release, asked only when configuration has not said.
    let through = match config.git.through {
        Some(through) => through,
        None => ask.choose_git()?,
    };

    let tag_pattern = config.tag_pattern_for(&project)?;
    let plan = app::bump::plan_from(
        fs,
        root,
        &project,
        Some(base),
        *transition,
        GitPlanning {
            through,
            commit_message: &config.git.commit_message,
            tag: &tag_pattern,
            tag_style: config.git.tag_style,
            tag_message: &config.git.tag_message,
        },
    )?;

    if !ask.confirm(&(guidance.summary)(&plan))? {
        return Ok(Guided::Declined);
    }

    let outcome = app::change::apply(fs, vcs, root, &plan.changes)?;
    Ok(Guided::Applied(Box::new(Application { plan, outcome })))
}

/// Determines the version to bump from, asking only if the files disagree.
fn resolve_base(ask: &dyn Interaction, files: &[FileVersion]) -> Result<Version, GuidedError> {
    let mut distinct: Vec<Version> = Vec::new();
    for file in files {
        if !distinct.contains(&file.version) {
            distinct.push(file.version.clone());
        }
    }

    match distinct.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(ChangeError::OutOfSync { found: Vec::new() }.into()),
        _ => {
            // Each candidate is shown with the files recording it, so the
            // choice is made on evidence rather than on a bare version string.
            let candidates: Vec<(String, String)> = distinct
                .iter()
                .map(|version| {
                    let where_seen = files
                        .iter()
                        .filter(|f| f.version == *version)
                        .map(|f| f.path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    (version.to_string(), where_seen)
                })
                .collect();

            let chosen = ask.choose_base(&candidates)?;
            chosen
                .parse()
                .map_err(|_| InteractionError::Cancelled.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{
        Answer, MemoryFileSystem, MemoryInteraction, MemoryVcs, Question, VcsCall,
    };
    use crate::config::GitThrough;

    const ROOT: &str = "/repo";

    fn config(text: &str) -> Config {
        Config::parse(Path::new("/repo/vump.toml"), text).expect("fixture parses")
    }

    /// A repository with one project recording `version`, tagging on bump.
    fn single(version: &str) -> (MemoryFileSystem, Config) {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", format!("{version}\n"));
        (
            fs,
            config("files = [\"VERSION\"]\n\n[git]\nthrough = \"tag\"\n"),
        )
    }

    fn guidance() -> Guidance<'static> {
        Guidance {
            project: None,
            branch: BranchCheck::Enforce,
            summary: |plan| plan.changes.target.to_string(),
            label: |transition| format!("{transition:?}"),
        }
    }

    fn drive(
        fs: &MemoryFileSystem,
        vcs: &MemoryVcs,
        ask: &MemoryInteraction,
        config: &Config,
        guidance: Guidance<'_>,
    ) -> Result<Guided, GuidedError> {
        run(fs, vcs, ask, Path::new(ROOT), config, guidance)
    }

    #[test]
    fn a_repository_with_one_project_is_not_asked_which() {
        // Every question a guided run can answer for itself is one it must not
        // ask: the value of the guided path is that it asks the minimum.
        let (fs, config) = single("1.2.3");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(false)]);

        drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert!(
            !ask.asked()
                .iter()
                .any(|q| matches!(q, Question::Project(_))),
            "{:?}",
            ask.asked()
        );
    }

    #[test]
    fn a_project_named_by_the_caller_is_not_asked_about() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/a/VERSION", "1.0.0\n")
            .with_file("/repo/b/VERSION", "2.0.0\n");
        let config = config(
            "[[project]]\nname = \"a\"\nfiles = [\"a/VERSION\"]\ntag_pattern = \"a-v{new_version}\"\n\n\
             [[project]]\nname = \"b\"\nfiles = [\"b/VERSION\"]\ntag_pattern = \"b-v{new_version}\"\n",
        );
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[
            Answer::Transition(0),
            Answer::Git(GitThrough::None),
            Answer::Confirm(false),
        ]);

        let mut guidance = guidance();
        guidance.project = Some("b");
        drive(&fs, &vcs, &ask, &config, guidance).unwrap();

        assert!(
            !ask.asked()
                .iter()
                .any(|q| matches!(q, Question::Project(_)))
        );
    }

    #[test]
    fn several_projects_with_none_named_are_asked_about() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/a/VERSION", "1.0.0\n")
            .with_file("/repo/b/VERSION", "2.0.0\n");
        let config = config(
            "[[project]]\nname = \"a\"\nfiles = [\"a/VERSION\"]\ntag_pattern = \"a-v{new_version}\"\n\n\
             [[project]]\nname = \"b\"\nfiles = [\"b/VERSION\"]\ntag_pattern = \"b-v{new_version}\"\n",
        );
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[
            Answer::Project("a".to_owned()),
            Answer::Transition(0),
            Answer::Git(GitThrough::None),
            Answer::Confirm(false),
        ]);

        drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert_eq!(
            ask.asked().first(),
            Some(&Question::Project(vec!["a".to_owned(), "b".to_owned()]))
        );
    }

    #[test]
    fn files_that_agree_are_not_asked_about() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.2.3\n")
            .with_file("/repo/sub/VERSION", "1.2.3\n");
        let config =
            config("files = [\"VERSION\", \"sub/VERSION\"]\n\n[git]\nthrough = \"none\"\n");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(false)]);

        drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert!(!ask.asked().iter().any(|q| matches!(q, Question::Base(_))));
    }

    #[test]
    fn files_that_disagree_ask_which_version_is_current() {
        // The disagreement is the whole reason the question exists, and the
        // answer decides what every later question means.
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.2.3\n")
            .with_file("/repo/sub/VERSION", "9.9.9\n");
        let config =
            config("files = [\"VERSION\", \"sub/VERSION\"]\n\n[git]\nthrough = \"none\"\n");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[
            Answer::Base("9.9.9".to_owned()),
            Answer::Transition(0),
            Answer::Confirm(true),
        ]);

        let result = drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert_eq!(
            ask.asked().first(),
            Some(&Question::Base(vec![
                "1.2.3".to_owned(),
                "9.9.9".to_owned()
            ]))
        );
        let Guided::Applied(applied) = result else {
            panic!("approved runs apply");
        };
        assert_eq!(applied.plan.from.to_string(), "9.9.9");
    }

    #[test]
    fn configured_git_settings_are_not_asked_about() {
        // A repository that has already said how far a release goes is never
        // asked again. That is what leaves two questions in the common case.
        let (fs, config) = single("1.2.3");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(false)]);

        drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert!(!ask.asked().contains(&Question::Git));
    }

    #[test]
    fn an_undeclared_git_step_is_asked_about() {
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");
        let config = config("files = [\"VERSION\"]\n");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[
            Answer::Transition(0),
            Answer::Git(GitThrough::Commit),
            Answer::Confirm(false),
        ]);

        drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert!(ask.asked().contains(&Question::Git));
    }

    #[test]
    fn declining_writes_nothing_and_touches_no_repository() {
        // The confirmation is the last point at which nothing has happened, so
        // it has to be the point at which nothing can.
        let (fs, config) = single("1.2.3");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(false)]);

        let result = drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert!(matches!(result, Guided::Declined));
        assert_eq!(fs.get("/repo/VERSION").as_deref(), Some("1.2.3\n"));
        assert!(
            !vcs.calls().iter().any(|c| matches!(c, VcsCall::Commit(_))),
            "{:?}",
            vcs.calls()
        );
    }

    #[test]
    fn approving_writes_the_files_and_tags() {
        let (fs, config) = single("1.2.3");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(true)]);

        let result = drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        let Guided::Applied(applied) = result else {
            panic!("approved runs apply");
        };
        assert_eq!(
            fs.get("/repo/VERSION").as_deref(),
            Some(format!("{}\n", applied.plan.changes.target).as_str())
        );
        assert!(
            vcs.calls().iter().any(|c| matches!(c, VcsCall::Tag(_, _))),
            "{:?}",
            vcs.calls()
        );
    }

    #[test]
    fn the_summary_shown_is_the_plan_that_would_run() {
        // The confirmation must describe the run being confirmed, not a
        // recomputed guess at it.
        let (fs, config) = single("1.2.3");
        let vcs = MemoryVcs::new();
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(true)]);

        let result = drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        let Guided::Applied(applied) = result else {
            panic!("approved runs apply");
        };
        assert!(
            ask.asked()
                .contains(&Question::Confirm(applied.plan.changes.target.to_string())),
            "{:?}",
            ask.asked()
        );
    }

    #[test]
    fn the_menu_marks_what_the_branch_policy_blocks() {
        // The guided path is the one this guard exists for, so the list has to
        // carry the answer rather than the refusal arriving afterwards.
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");
        let config = config(
            "files = [\"VERSION\"]\n\n[git]\nthrough = \"tag\"\nrelease_branches = [\"main\"]\n",
        );
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(false)]);

        drive(&fs, &vcs, &ask, &config, guidance()).unwrap_err();

        let offered = ask.offered().expect("the bump is still asked");
        let stable = offered
            .iter()
            .filter(|o| !o.result.contains('-'))
            .collect::<Vec<_>>();
        assert_ne!(stable.len(), 0);
        assert!(
            stable
                .iter()
                .all(|o| o.availability == Availability::Blocked),
            "{offered:?}"
        );
        assert!(
            offered.iter().any(|o| o.availability == Availability::Free),
            "{offered:?}"
        );
    }

    #[test]
    fn a_policy_blocking_everything_refuses_before_asking() {
        // A menu with nothing selectable is a dead end, so it is never shown.
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");
        let config = config(
            "files = [\"VERSION\"]\n\n[git]\nthrough = \"tag\"\n\
             release_branches = [\"main\"]\nprerelease_branches = [\"main\"]\n",
        );
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let ask = MemoryInteraction::new(&[]);

        let error = drive(&fs, &vcs, &ask, &config, guidance()).unwrap_err();

        assert!(
            matches!(error, GuidedError::Change(ChangeError::OffBranch { .. })),
            "{error:?}"
        );
        assert!(ask.offered().is_none(), "{:?}", ask.asked());
    }

    #[test]
    fn waiving_the_policy_marks_the_bumps_and_says_so() {
        // --any-branch restores the choice and keeps the hazard visible, which
        // is the whole difference between waiving a guard and removing it.
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");
        let config = config(
            "files = [\"VERSION\"]\n\n[git]\nthrough = \"tag\"\nrelease_branches = [\"main\"]\n",
        );
        let vcs = MemoryVcs::new().on_branch("feat/x");
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(false)]);

        let mut guidance = guidance();
        guidance.branch = BranchCheck::Skipped;
        drive(&fs, &vcs, &ask, &config, guidance).unwrap();

        let offered = ask.offered().expect("every bump is still offered");
        assert!(
            offered
                .iter()
                .any(|o| o.availability == Availability::Warned),
            "{offered:?}"
        );
        assert!(
            ask.notices().iter().any(|n| n.contains("feat/x")),
            "{:?}",
            ask.notices()
        );
    }

    #[test]
    fn a_listed_branch_is_never_mentioned() {
        // Nothing to say is said: a repository whose policy is satisfied looks
        // exactly like one that has none.
        let fs = MemoryFileSystem::new().with_file("/repo/VERSION", "1.2.3\n");
        let config = config(
            "files = [\"VERSION\"]\n\n[git]\nthrough = \"tag\"\nrelease_branches = [\"main\"]\n",
        );
        let vcs = MemoryVcs::new().on_branch("main");
        let ask = MemoryInteraction::new(&[Answer::Transition(0), Answer::Confirm(false)]);

        drive(&fs, &vcs, &ask, &config, guidance()).unwrap();

        assert!(ask.notices().is_empty(), "{:?}", ask.notices());
        assert!(
            ask.offered()
                .expect("asked")
                .iter()
                .all(|o| o.availability == Availability::Free)
        );
    }
}
