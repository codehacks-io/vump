//! Command-line interface.
//!
//! This layer owns argument parsing, adapter selection, and presentation. It
//! contains no rules of its own: it translates a parsed command into a use
//! case, then renders whatever comes back.

pub mod exit;
pub mod render;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use thiserror::Error;

use crate::adapters::{GitCli, GitHubReleases, RealFileSystem, TerminalInteraction};
use crate::app::change::{BranchCheck, ChangeError, GitPlanning, Nesting};
use crate::app::update::Channel;
use crate::app::{self, AppError};
use crate::config::{Config, ConfigError, GitThrough, TagStyle};
use crate::domain::{PreLabel, StableBump, Transition, TransitionError};
use crate::ports::{FsError, InteractionError};

use exit::Exit;

/// Keep semver version numbers in sync across a repository.
///
/// `about` and `long_about` are set explicitly rather than inherited from this
/// doc comment: doc comments target maintainers, whereas help text targets
/// users, and the two should be free to diverge.
#[derive(Debug, Parser)]
#[command(
    name = "vump",
    version,
    about = "Keep semver version numbers in sync across a repository",
    long_about = "Keep semver version numbers in sync across the files of a repository, \
                  and verify in CI that a released tag matches what is recorded in source.\n\n\
                  Run without a subcommand to be guided through a bump. Naming a subcommand \
                  runs without prompting, always."
)]
pub struct Cli {
    /// Omitted to run interactively.
    #[command(subcommand)]
    command: Option<Command>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    /// Select a project in a repository that declares several.
    #[arg(long, global = true, value_name = "NAME")]
    project: Option<String>,

    /// Act on a vump.toml nested inside another's repository.
    ///
    /// Global because every command touching a project is refused from one,
    /// reading included: the hazard is using the wrong configuration, not
    /// writing through it.
    #[arg(long, global = true)]
    allow_nested: bool,

    /// Release from a branch the configuration does not list.
    ///
    /// Global because the branches a release may come from is a property of
    /// the repository, not of the command that reaches a tag.
    #[arg(long, global = true)]
    any_branch: bool,
}

/// The operation to perform.
///
/// Every bump type is a real subcommand rather than a matched string, so that
/// help text, validation, and typo diagnostics come from one source.
///
/// Naming a bump subcommand is what makes a run non-interactive. Anything a
/// subcommand would otherwise have to ask about is a required flag or an
/// error, never a prompt.
#[derive(Debug, Subcommand)]
enum Command {
    /// Bump the patch component (1.2.3 -> 1.2.4).
    Patch(BumpArgs),

    /// Bump the minor component (1.2.3 -> 1.3.0).
    Minor(BumpArgs),

    /// Bump the major component (1.2.3 -> 2.0.0).
    Major(BumpArgs),

    /// Start or advance an alpha pre-release.
    Alpha(PreReleaseArgs),

    /// Start or advance a beta pre-release.
    Beta(PreReleaseArgs),

    /// Start or advance a release candidate.
    Rc(PreReleaseArgs),

    /// Drop the pre-release suffix (1.2.3-rc.1 -> 1.2.3).
    Release(BumpArgs),

    /// Verify that tracked files record the given version.
    ///
    /// Intended for CI: run it against a pushed tag to abort before any build
    /// or publish work when the tag disagrees with source. A leading "v" is
    /// accepted and ignored.
    Check {
        /// Version to verify, with or without a leading "v".
        version: String,
    },

    /// Write an exact version to every tracked file.
    ///
    /// Unlike a bump this does not require the files to agree first, which is
    /// how a disagreement is repaired, and it does not refuse to move
    /// backwards: naming a version is consent.
    Set {
        /// Version to write, with or without a leading "v".
        version: String,

        /// Report what would change without writing anything.
        #[arg(long)]
        dry_run: bool,

        #[command(flatten)]
        git: GitArgs,
    },

    /// Push the commit and the one tag a bump already created.
    ///
    /// For finishing a bump that stopped at `tag`. `git push --follow-tags`
    /// does the same job but pushes every annotated tag reachable from `HEAD`,
    /// which in a repository tagging several projects independently can
    /// publish another project's release. This names one ref.
    Push {
        /// Report what would be pushed without pushing it.
        #[arg(long)]
        dry_run: bool,
    },

    /// Report the versions currently recorded, and whether they agree.
    Status,

    /// Create a vump.toml tracking every version file found here.
    ///
    /// Writes a configuration that keeps all discovered files at one version,
    /// with git actions off. Edit the result to split the repository into
    /// independently-versioned projects or to enable git actions.
    Init {
        /// Overwrite an existing vump.toml.
        #[arg(long)]
        force: bool,
    },

    /// Manage this installation of vump.
    ///
    /// These act on the binary rather than on a project, and so need no
    /// vump.toml.
    ///
    /// The variant cannot be called `Self`, which is a reserved word, so the
    /// command name is set explicitly rather than derived from it.
    #[command(subcommand, name = "self")]
    SelfCmd(SelfCommand),
}

/// Operations on the vump installation itself.
#[derive(Debug, Subcommand)]
enum SelfCommand {
    /// Replace this binary with a published release.
    Update {
        /// Install this exact version, newer or older.
        ///
        /// Naming a version bypasses the channel and the refusal to downgrade,
        /// which is how a rollback is expressed.
        #[arg(long, value_name = "VERSION")]
        to: Option<String>,

        /// Least mature kind of release to accept.
        #[arg(long, value_name = "CHANNEL", default_value = "stable")]
        channel: ChannelArg,
    },

    /// Report the running version and whether a newer one is published.
    Status {
        /// Least mature kind of release to consider.
        #[arg(long, value_name = "CHANNEL", default_value = "stable")]
        channel: ChannelArg,
    },

    /// List published releases, marking the running one.
    List {
        /// Least mature kind of release to include.
        #[arg(long, value_name = "CHANNEL", default_value = "stable")]
        channel: ChannelArg,

        /// Show at most this many, newest first.
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
}

/// The least mature kind of release to accept.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ChannelArg {
    /// Finished releases only.
    Stable,
    /// Release candidates and finished releases.
    Rc,
    /// Betas and anything more mature.
    Beta,
    /// Everything, including alphas.
    Alpha,
}

impl From<ChannelArg> for Channel {
    fn from(arg: ChannelArg) -> Self {
        match arg {
            ChannelArg::Stable => Self::Stable,
            ChannelArg::Rc => Self::Rc,
            ChannelArg::Beta => Self::Beta,
            ChannelArg::Alpha => Self::Alpha,
        }
    }
}

/// Options shared by every bump.
#[derive(Debug, clap::Args)]
struct BumpArgs {
    /// Report what would change without writing anything.
    #[arg(long)]
    dry_run: bool,

    #[command(flatten)]
    git: GitArgs,
}

/// Options for starting or advancing a pre-release.
#[derive(Debug, clap::Args)]
struct PreReleaseArgs {
    /// Which release the pre-release leads to.
    ///
    /// Required when the current version is stable, because a pre-release must
    /// know which future release it precedes. Ignored when already on one.
    #[arg(long, value_name = "BUMP")]
    toward: Option<StableBumpArg>,

    /// Report what would change without writing anything.
    #[arg(long)]
    dry_run: bool,

    #[command(flatten)]
    git: GitArgs,
}

/// Git settings overridable for a single run.
///
/// Each mirrors a `[git]` key by name and value, and replaces it rather than
/// adding to it, so a configured step can be lowered as readily as raised.
#[derive(Debug, clap::Args)]
struct GitArgs {
    /// How far to carry the release, overriding vump.toml for this run.
    #[arg(long, value_name = "STEP")]
    through: Option<GitThroughArg>,

    /// How the tag object is written, overriding vump.toml for this run.
    #[arg(long, value_name = "STYLE")]
    tag_style: Option<TagStyleArg>,
}

/// How far a bump carries the release.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum GitThroughArg {
    /// Write the files and stop.
    None,
    /// Commit the changed files.
    Commit,
    /// Commit, then tag.
    Tag,
    /// Commit, tag, then push the commit and that tag.
    Push,
}

impl From<GitThroughArg> for GitThrough {
    fn from(arg: GitThroughArg) -> Self {
        match arg {
            GitThroughArg::None => Self::None,
            GitThroughArg::Commit => Self::Commit,
            GitThroughArg::Tag => Self::Tag,
            GitThroughArg::Push => Self::Push,
        }
    }
}

/// How a tag object is written.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum TagStyleArg {
    /// A tag object carrying a message, a tagger, and a date.
    Annotated,
    /// A bare pointer to a commit.
    Lightweight,
    /// An annotated tag, signed.
    Signed,
}

impl From<TagStyleArg> for TagStyle {
    fn from(arg: TagStyleArg) -> Self {
        match arg {
            TagStyleArg::Annotated => Self::Annotated,
            TagStyleArg::Lightweight => Self::Lightweight,
            TagStyleArg::Signed => Self::Signed,
        }
    }
}

/// The stable bump a pre-release is based on.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum StableBumpArg {
    /// 1.2.3 -> 1.2.4
    Patch,
    /// 1.2.3 -> 1.3.0
    Minor,
    /// 1.2.3 -> 2.0.0
    Major,
}

impl From<StableBumpArg> for StableBump {
    fn from(arg: StableBumpArg) -> Self {
        match arg {
            StableBumpArg::Patch => Self::Patch,
            StableBumpArg::Minor => Self::Minor,
            StableBumpArg::Major => Self::Major,
        }
    }
}

impl GitArgs {
    /// How far to carry the release: the flag when given, otherwise
    /// configuration.
    ///
    /// A subcommand never prompts, so configuration that decides nothing means
    /// no git work rather than a question.
    fn through(&self, settings: &crate::config::GitSettings) -> GitThrough {
        self.through
            .map(GitThrough::from)
            .or(settings.through)
            .unwrap_or(GitThrough::None)
    }

    /// How the tag object is written: the flag when given, otherwise
    /// configuration.
    fn tag_style(&self, settings: &crate::config::GitSettings) -> TagStyle {
        self.tag_style.map_or(settings.tag_style, TagStyle::from)
    }
}

/// Parses arguments, runs the requested command, and returns a process status.
#[must_use]
pub fn run() -> ExitCode {
    let cli = Cli::parse();

    match execute(&cli) {
        Ok(exit) => exit.into(),
        Err(error) => {
            let exit = error.exit();
            render::error(&error.to_string(), exit, cli.json);
            exit.into()
        }
    }
}

/// Everything a command needs from the environment, resolved once.
struct Context {
    fs: RealFileSystem,
    root: std::path::PathBuf,
    config: Config,
    json: bool,
    project: Option<String>,
    branch: BranchCheck,
}

fn execute(cli: &Cli) -> Result<Exit, CliError> {
    let cwd = std::env::current_dir().map_err(|e| CliError::WorkingDirectory(e.to_string()))?;

    let nesting = if cli.allow_nested {
        Nesting::Allowed
    } else {
        Nesting::Refuse
    };

    // init and update operate on the installation rather than on a project, so
    // they run before configuration is looked for. init exists precisely
    // because there is none yet.
    match &cli.command {
        Some(Command::Init { force }) => {
            // Refused here too, and this is the place it matters most: writing
            // a configuration under one that already exists is what creates
            // the arrangement every other command then has to refuse.
            app::change::check_nesting(&RealFileSystem, &cwd, nesting)?;
            let written = app::init::init(&RealFileSystem, &cwd, *force)?;
            render::init(&written, cli.json);
            return Ok(Exit::Success);
        }
        Some(Command::SelfCmd(command)) => return self_command(command, cli.json),
        _ => {}
    }

    let (root, config) = app::discover(&RealFileSystem, &cwd)?;

    // Every command acting on a project is covered, before any of them runs.
    // One gate at the point configuration becomes known, rather than a check
    // per use case that a later command could be written without.
    app::change::check_nesting(&RealFileSystem, &root, nesting)?;

    let ctx = Context {
        fs: RealFileSystem,
        root,
        config,
        json: cli.json,
        project: cli.project.clone(),
        branch: if cli.any_branch {
            BranchCheck::Skipped
        } else {
            BranchCheck::Enforce
        },
    };

    let pre = |label, args: &PreReleaseArgs| Transition::PreRelease {
        label,
        toward: args.toward.map(Into::into),
    };

    let Some(command) = &cli.command else {
        return interactive(&ctx);
    };

    match command {
        Command::Patch(a) => bump(
            &ctx,
            Transition::Stable(StableBump::Patch),
            a.dry_run,
            &a.git,
        ),
        Command::Minor(a) => bump(
            &ctx,
            Transition::Stable(StableBump::Minor),
            a.dry_run,
            &a.git,
        ),
        Command::Major(a) => bump(
            &ctx,
            Transition::Stable(StableBump::Major),
            a.dry_run,
            &a.git,
        ),
        Command::Release(a) => bump(&ctx, Transition::Release, a.dry_run, &a.git),
        Command::Alpha(a) => bump(&ctx, pre(PreLabel::Alpha, a), a.dry_run, &a.git),
        Command::Beta(a) => bump(&ctx, pre(PreLabel::Beta, a), a.dry_run, &a.git),
        Command::Rc(a) => bump(&ctx, pre(PreLabel::Rc, a), a.dry_run, &a.git),
        Command::Check { version } => check(&ctx, version),
        Command::Set {
            version,
            dry_run,
            git,
        } => set(&ctx, version, *dry_run, git),
        Command::Push { dry_run } => push(&ctx, *dry_run),
        Command::Status => status(&ctx),
        // Handled before configuration discovery.
        Command::Init { .. } | Command::SelfCmd(_) => Ok(Exit::Success),
    }
}

/// The version of the running binary.
///
/// Taken from the manifest it was built from, which is the same value `check`
/// verifies against the release tag.
fn running_version() -> Result<semver::Version, CliError> {
    env!("CARGO_PKG_VERSION")
        .parse()
        .map_err(|e: semver::Error| CliError::InvalidVersionArgument {
            text: env!("CARGO_PKG_VERSION").to_owned(),
            detail: e.to_string(),
        })
}

/// Runs a command that acts on the installation rather than on a project.
fn self_command(command: &SelfCommand, json: bool) -> Result<Exit, CliError> {
    let current = running_version()?;
    let source = GitHubReleases::new();

    match command {
        SelfCommand::Status { channel } => {
            let outcome = app::update::status(&source, &current, (*channel).into())?;
            render::update(&outcome, json);

            // Scripts ask this to decide whether to act, so the answer is
            // carried by the exit status as well as by the output.
            Ok(match outcome {
                app::update::UpdateOutcome::Available { .. } => Exit::Failure,
                _ => Exit::Success,
            })
        }

        SelfCommand::List { channel, limit } => {
            let listing = app::update::list(&source, &current, (*channel).into())?;
            render::releases(&listing, *limit, json);
            Ok(Exit::Success)
        }

        SelfCommand::Update { to, channel } => {
            let requested = to
                .as_deref()
                .map(|text| {
                    app::check::parse_expected(text).map_err(|e| CliError::InvalidVersionArgument {
                        text: text.to_owned(),
                        detail: e.to_string(),
                    })
                })
                .transpose()?;

            let outcome = app::update::update(
                &source,
                &current,
                (std::env::consts::OS, std::env::consts::ARCH),
                (*channel).into(),
                requested.as_ref(),
            )?;

            render::update(&outcome, json);
            Ok(Exit::Success)
        }
    }
}

/// Guides a bump, asking only what has not already been decided.
///
/// The run itself lives in `app::guided`, driven entirely through ports. What
/// remains here is what belongs to a terminal: which adapters to build, how a
/// plan is worded, and what to print about the result.
fn interactive(ctx: &Context) -> Result<Exit, CliError> {
    let ask = TerminalInteraction::new();
    let vcs = GitCli::new(&ctx.root);

    let guided = app::guided::run(
        &ctx.fs,
        &vcs,
        &ask,
        &ctx.root,
        &ctx.config,
        app::guided::Guidance {
            project: ctx.project.as_deref(),
            branch: ctx.branch,
            summary: render::summary,
            label: describe_transition,
        },
    )?;

    match guided {
        app::guided::Guided::Declined => {
            println!("Nothing was changed.");
            Ok(Exit::Success)
        }
        app::guided::Guided::Applied(applied) => {
            let (plan, outcome) = (applied.plan, applied.outcome);
            render::applied(&plan.changes, &outcome, ctx.json);

            // A failed push leaves real, recoverable work behind, so it is
            // reported as a git failure rather than as a successful run.
            Ok(if outcome.push_error.is_some() {
                Exit::Git
            } else {
                Exit::Success
            })
        }
    }
}

/// Names a transition the way the corresponding subcommand is spelled, so the
/// menu teaches the non-interactive equivalent.
fn describe_transition(transition: Transition) -> String {
    match transition {
        Transition::Stable(bump) => bump.to_string(),
        Transition::PreRelease {
            label,
            toward: Some(bump),
        } => format!("{label} --toward {bump}"),
        Transition::PreRelease {
            label,
            toward: None,
        } => label.to_string(),
        Transition::Release => "release".to_owned(),
    }
}

fn bump(
    ctx: &Context,
    transition: Transition,
    dry_run: bool,
    git_args: &GitArgs,
) -> Result<Exit, CliError> {
    let project = ctx.config.select(ctx.project.as_deref())?;
    let tag_pattern = ctx.config.tag_pattern_for(project)?;

    let plan = app::bump::plan(
        &ctx.fs,
        &ctx.root,
        project,
        transition,
        GitPlanning {
            through: git_args.through(&ctx.config.git),
            commit_message: &ctx.config.git.commit_message,
            tag: &tag_pattern,
            tag_style: git_args.tag_style(&ctx.config.git),
            tag_message: &ctx.config.git.tag_message,
        },
    )?;

    // Before the dry run reports and before anything is written: a plan for a
    // run that would be refused is not a plan.
    let vcs = GitCli::new(&ctx.root);
    if let Some(off) = app::change::check_branch(
        &vcs,
        &plan.changes.target,
        plan.changes.git.touches_repository(),
        &ctx.config.git,
        ctx.branch,
    )? {
        render::off_branch(&off, ctx.branch);
    }

    if dry_run {
        render::plan(&plan.changes, ctx.json);
        return Ok(Exit::Success);
    }

    let outcome = app::change::apply(&ctx.fs, &vcs, &ctx.root, &plan.changes)?;
    render::applied(&plan.changes, &outcome, ctx.json);

    // A failed push leaves real, recoverable work behind, so it is reported as
    // a git failure rather than as a successful run.
    Ok(if outcome.push_error.is_some() {
        Exit::Git
    } else {
        Exit::Success
    })
}

/// Verifies that a project's files record the version a tag claims.
///
/// The argument may be a full tag (`api-v1.2.3`) or a bare version (`1.2.3`).
/// A tag identifies its own project, which is what lets a CI job pass
/// `github.ref_name` straight through without knowing which project it names.
/// Writes an exact version to every tracked file.
fn set(ctx: &Context, version: &str, dry_run: bool, git_args: &GitArgs) -> Result<Exit, CliError> {
    let target =
        app::check::parse_expected(version).map_err(|e| CliError::InvalidVersionArgument {
            text: version.to_owned(),
            detail: e.to_string(),
        })?;

    let project = ctx.config.select(ctx.project.as_deref())?;
    let tag_pattern = ctx.config.tag_pattern_for(project)?;

    let changes = app::set::set(
        &ctx.fs,
        &ctx.root,
        project,
        target,
        GitPlanning {
            through: git_args.through(&ctx.config.git),
            commit_message: &ctx.config.git.commit_message,
            tag: &tag_pattern,
            tag_style: git_args.tag_style(&ctx.config.git),
            tag_message: &ctx.config.git.tag_message,
        },
    )?;

    let vcs = GitCli::new(&ctx.root);
    if let Some(off) = app::change::check_branch(
        &vcs,
        &changes.target,
        changes.git.touches_repository(),
        &ctx.config.git,
        ctx.branch,
    )? {
        render::off_branch(&off, ctx.branch);
    }

    if dry_run {
        render::plan(&changes, ctx.json);
        return Ok(Exit::Success);
    }

    // Writing the version the files already record changes nothing, and git
    // refuses an empty commit, so the run stops here rather than failing at the
    // commit for a reason that has nothing to do with what was asked.
    if !changes.changes_anything() {
        render::unchanged(&changes, ctx.json);
        return Ok(Exit::Success);
    }

    let outcome = app::change::apply(&ctx.fs, &vcs, &ctx.root, &changes)?;
    render::applied(&changes, &outcome, ctx.json);

    Ok(if outcome.push_error.is_some() {
        Exit::Git
    } else {
        Exit::Success
    })
}

fn check(ctx: &Context, version: &str) -> Result<Exit, CliError> {
    let (project, expected) =
        if let Some((project, from_tag)) = ctx.config.project_for_tag(version)? {
            // An explicit selection must not silently disagree with the tag.
            if let Some(requested) = ctx.project.as_deref() {
                let selected = ctx.config.select(Some(requested))?;
                if selected.name != project.name {
                    return Err(CliError::TagProjectMismatch {
                        tag: version.to_owned(),
                        from_tag: project.name.clone().unwrap_or_default(),
                        requested: requested.to_owned(),
                    });
                }
            }
            (project, from_tag)
        } else {
            // Not a tag this repository would produce, so it is read as a bare
            // version and the project selected the usual way.
            let expected = app::check::parse_expected(version).map_err(|e| {
                // Reporting only "not a version" is unhelpful for something that
                // clearly looks like a tag, so the patterns that were tried are
                // named.
                let patterns = ctx.config.tag_patterns();
                if version.contains('-') && !patterns.is_empty() {
                    CliError::UnrecognizedTag {
                        tag: version.to_owned(),
                        patterns: patterns.join(", "),
                    }
                } else {
                    CliError::InvalidVersionArgument {
                        text: version.to_owned(),
                        detail: e.to_string(),
                    }
                }
            })?;
            (ctx.config.select(ctx.project.as_deref())?, expected)
        };

    let report = app::check::check(&ctx.fs, &ctx.root, project, expected)?;

    render::check(&report, ctx.json);

    Ok(if report.is_satisfied() {
        Exit::Success
    } else {
        Exit::VersionMismatch
    })
}

/// Pushes the commit and the one tag a bump already created.
fn push(ctx: &Context, dry_run: bool) -> Result<Exit, CliError> {
    let project = ctx.config.select(ctx.project.as_deref())?;

    let vcs = GitCli::new(&ctx.root);
    let planned = app::push::plan(&ctx.fs, &vcs, &ctx.root, &ctx.config, project)?;

    // The branch policy governs this as it governs a bump. A bump that stopped
    // at `tag` was never checked against it — nothing had left the machine —
    // and this is the step that would put the tag on the remote, so the check
    // belongs here too, against the version already recorded.
    if let Some(off) =
        app::change::check_branch(&vcs, &planned.version, true, &ctx.config.git, ctx.branch)?
    {
        render::off_branch(&off, ctx.branch);
    }

    if !dry_run {
        app::push::push(&ctx.fs, &vcs, &ctx.root, &ctx.config, project)?;
    }

    render::pushed(&planned, dry_run, ctx.json);
    Ok(Exit::Success)
}

fn status(ctx: &Context) -> Result<Exit, CliError> {
    // Selecting a project narrows the report; without one, every declared
    // project is reported.
    let scoped;
    let target = match ctx.project.as_deref() {
        Some(name) => {
            scoped = Config {
                git: ctx.config.git.clone(),
                projects: vec![ctx.config.select(Some(name))?.clone()],
            };
            &scoped
        }
        None => &ctx.config,
    };

    let statuses = app::status::status(&ctx.fs, &ctx.root, target)?;
    render::status(&statuses, ctx.json);

    Ok(
        if statuses.iter().all(app::status::ProjectStatus::is_in_sync) {
            Exit::Success
        } else {
            Exit::OutOfSync
        },
    )
}

impl From<app::guided::GuidedError> for CliError {
    /// Unwraps rather than nesting, so a guided failure exits with the same
    /// code the same failure would exit with from a subcommand.
    fn from(error: app::guided::GuidedError) -> Self {
        use app::guided::GuidedError as Guided;
        match error {
            Guided::Config(e) => Self::Config(e),
            Guided::App(e) => Self::App(e),
            Guided::Change(e) => Self::Change(e),
            Guided::Transition(e) => Self::Transition(e),
            Guided::Interaction(e) => Self::Interaction(e),
        }
    }
}

/// Maps a use-case failure to its documented exit code.
///
/// An unreadable file is an environment failure; every other way a declared
/// file fails to yield a version means configuration points at something
/// unusable.
fn app_exit(error: &AppError) -> Exit {
    match error {
        AppError::Filesystem(FsError::Io { .. }) => Exit::Failure,
        AppError::MissingFile { .. }
        | AppError::Filesystem(FsError::NotFound { .. })
        | AppError::VersionFile(_)
        | AppError::UndeclaredLock { .. } => Exit::Config,
    }
}

/// A failure that prevents a command from completing.
#[derive(Debug, Error)]
enum CliError {
    #[error("{0}")]
    Config(#[from] ConfigError),

    #[error("{0}")]
    App(#[from] AppError),

    #[error("{0}")]
    Change(#[from] ChangeError),

    #[error("{0}")]
    Push(#[from] app::push::PushError),

    #[error("{0}")]
    Transition(#[from] TransitionError),

    #[error("{0}")]
    Interaction(#[from] InteractionError),

    #[error("{0}")]
    Init(#[from] app::init::InitError),

    #[error("{0}")]
    Update(#[from] app::update::UpdateError),

    #[error("{text:?} is not a valid version: {detail}")]
    InvalidVersionArgument { text: String, detail: String },

    #[error("tag {tag:?} belongs to project {from_tag:?}, but --project {requested:?} was given")]
    TagProjectMismatch {
        tag: String,
        from_tag: String,
        requested: String,
    },

    #[error("{tag:?} is neither a version nor a tag any project produces ({patterns})")]
    UnrecognizedTag { tag: String, patterns: String },

    #[error("cannot determine the current directory: {0}")]
    WorkingDirectory(String),
}

impl CliError {
    /// Maps a failure to its documented exit code.
    fn exit(&self) -> Exit {
        match self {
            Self::InvalidVersionArgument { .. }
            | Self::TagProjectMismatch { .. }
            | Self::UnrecognizedTag { .. } => Exit::Usage,
            Self::Config(_) => Exit::Config,
            Self::App(app) => app_exit(app),
            Self::Push(push) => match push {
                app::push::PushError::Config(_) => Exit::Config,
                app::push::PushError::App(app) => app_exit(app),
                app::push::PushError::OutOfSync { .. } => Exit::OutOfSync,
                // Asking to push a tag that was never made is a mistake in the
                // command, not a failure of the repository.
                app::push::PushError::NoTag { .. } => Exit::Usage,
                app::push::PushError::Vcs(_) => Exit::Git,
            },
            Self::Transition(_) => Exit::InvalidTransition,
            // Updating touches the installation rather than a project, so
            // nothing about it maps onto the project-shaped codes. Neither does
            // failing to resolve the working directory.
            Self::WorkingDirectory(_) => Exit::Failure,
            Self::Update(update) => match update {
                // A caller seeing this should investigate rather than retry.
                app::update::UpdateError::Unverifiable { .. }
                | app::update::UpdateError::Corrupt(_) => Exit::Unverifiable,
                // These may succeed on a retry, or with a different version.
                app::update::UpdateError::Unreachable { .. }
                | app::update::UpdateError::NoSuchRelease { .. }
                | app::update::UpdateError::UnsupportedPlatform { .. } => Exit::ReleaseUnavailable,
                app::update::UpdateError::UnknownChannel(_) => Exit::Usage,
                app::update::UpdateError::InstallFailed { .. } => Exit::Failure,
            },
            Self::Init(init) => match init {
                app::init::InitError::Filesystem(FsError::Io { .. }) => Exit::Failure,
                app::init::InitError::AlreadyExists { .. }
                | app::init::InitError::NothingFound { .. }
                | app::init::InitError::Filesystem(FsError::NotFound { .. }) => Exit::Config,
            },
            Self::Interaction(interaction) => match interaction {
                // Declining is a decision, not a fault, but nothing was done,
                // so it must not look like a successful run either.
                InteractionError::Cancelled => Exit::Failure,
                // Reaching a prompt with nobody to answer it means the command
                // was invoked the wrong way for its environment.
                InteractionError::Unavailable { .. } => Exit::Usage,
            },
            Self::Change(change) => match change {
                ChangeError::App(app) => app_exit(app),
                ChangeError::Transition(_) => Exit::InvalidTransition,
                ChangeError::OutOfSync { .. } => Exit::OutOfSync,
                ChangeError::DirtyTree { .. } => Exit::DirtyTree,
                ChangeError::NestedConfig { .. }
                | ChangeError::OffBranch { .. }
                | ChangeError::DetachedHead => Exit::Config,
                ChangeError::Vcs(_) => Exit::Git,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        // Catches conflicting flags, duplicate names and malformed help at test
        // time rather than on first run.
        Cli::command().debug_assert();
    }

    #[test]
    fn check_accepts_a_version_argument() {
        let cli = Cli::try_parse_from(["vump", "check", "v1.2.3"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Check { version }) if version == "v1.2.3"
        ));
    }

    #[test]
    fn omitting_a_subcommand_selects_the_interactive_path() {
        let cli = Cli::try_parse_from(["vump"]).expect("bare vump must parse");
        assert!(cli.command.is_none());
    }

    #[test]
    fn every_bump_subcommand_is_reachable() {
        for name in ["patch", "minor", "major", "alpha", "beta", "rc", "release"] {
            Cli::try_parse_from(["vump", name])
                .unwrap_or_else(|e| panic!("{name} must be a subcommand: {e}"));
        }
    }

    #[test]
    fn a_step_beyond_the_ladder_is_refused_at_parse_time() {
        // Naming one step is what makes a contradictory pair unspellable, so
        // the only way left to get it wrong is to name a step that is not one.
        assert!(Cli::try_parse_from(["vump", "patch", "--through", "tag"]).is_ok());
        assert!(Cli::try_parse_from(["vump", "patch", "--through", "everything"]).is_err());
    }

    #[test]
    fn toward_only_accepts_stable_bumps() {
        assert!(Cli::try_parse_from(["vump", "alpha", "--toward", "minor"]).is_ok());
        assert!(Cli::try_parse_from(["vump", "alpha", "--toward", "beta"]).is_err());
    }

    /// Every long flag clap knows about, from the root and every subcommand.
    fn every_long_flag(command: &clap::Command, into: &mut Vec<String>) {
        for arg in command.get_arguments() {
            if let Some(long) = arg.get_long() {
                into.push(format!("--{long}"));
            }
        }
        for sub in command.get_subcommands() {
            every_long_flag(sub, into);
        }
    }

    #[test]
    fn the_documented_flag_tables_name_every_flag() {
        // The tables are written by hand and drifted: `vump init --force`
        // shipped, was referred to in prose, and appeared in neither table.
        let mut flags = Vec::new();
        every_long_flag(&Cli::command(), &mut flags);
        flags.retain(|f| f != "--help" && f != "--version");
        flags.sort_unstable();
        flags.dedup();
        assert!(!flags.is_empty(), "no flags found to check");

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for doc in ["DESIGN.md", "README.md"] {
            let text = std::fs::read_to_string(root.join(doc)).expect("document is readable");
            for flag in &flags {
                assert!(
                    text.contains(&format!("`{flag}")),
                    "{doc} never mentions {flag}",
                );
            }
        }
    }

    #[test]
    fn every_menu_label_is_a_command_that_parses() {
        // The menu's labels are the non-interactive equivalents, so a label
        // that does not parse teaches an invocation that does not exist. That
        // is invisible to every other test: the guided path never runs them.
        for current in ["1.2.3", "1.2.3-alpha.0", "1.2.3-beta.1", "1.2.3-rc.2"] {
            let version = current.parse().expect("a fixture version parses");
            for (transition, _) in
                crate::domain::bump::valid_transitions(&version).expect("the fixture is offerable")
            {
                let label = describe_transition(transition);
                let argv = std::iter::once("vump").chain(label.split_whitespace());
                Cli::try_parse_from(argv).unwrap_or_else(|e| {
                    panic!("menu offers {label:?} from {current}, which does not parse: {e}")
                });
            }
        }
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        let cli = Cli::try_parse_from(["vump", "check", "1.0.0", "--json"]).unwrap();
        assert!(cli.json);

        let cli = Cli::try_parse_from(["vump", "--json", "status"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn project_selection_is_available_to_every_command() {
        let cli = Cli::try_parse_from(["vump", "status", "--project", "api"]).unwrap();
        assert_eq!(cli.project.as_deref(), Some("api"));
    }

    #[test]
    fn check_requires_a_version() {
        assert!(Cli::try_parse_from(["vump", "check"]).is_err());
    }

    #[test]
    fn an_unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["vump", "bogus"]).is_err());
    }

    #[test]
    fn installation_commands_live_under_self() {
        for args in [
            vec!["vump", "self", "update"],
            vec!["vump", "self", "status"],
            vec!["vump", "self", "list"],
        ] {
            Cli::try_parse_from(&args).unwrap_or_else(|e| panic!("{args:?} must parse: {e}"));
        }

        // `self` alone is a group, not an action.
        assert!(Cli::try_parse_from(["vump", "self"]).is_err());
    }

    #[test]
    fn every_channel_is_accepted_and_others_are_not() {
        for channel in ["stable", "rc", "beta", "alpha"] {
            Cli::try_parse_from(["vump", "self", "status", "--channel", channel])
                .unwrap_or_else(|e| panic!("{channel} must parse: {e}"));
        }
        assert!(Cli::try_parse_from(["vump", "self", "status", "--channel", "nightly"]).is_err());
    }

    #[test]
    fn the_channel_defaults_to_stable() {
        let cli = Cli::try_parse_from(["vump", "self", "status"]).unwrap();
        let Some(Command::SelfCmd(SelfCommand::Status { channel })) = cli.command else {
            panic!("expected self status");
        };
        assert!(matches!(channel, ChannelArg::Stable));
    }

    #[test]
    fn an_exact_version_can_be_requested() {
        let cli = Cli::try_parse_from(["vump", "self", "update", "--to", "1.2.3"]).unwrap();
        let Some(Command::SelfCmd(SelfCommand::Update { to, .. })) = cli.command else {
            panic!("expected self update");
        };
        assert_eq!(to.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn listing_accepts_a_limit() {
        let cli = Cli::try_parse_from(["vump", "self", "list", "--limit", "5"]).unwrap();
        let Some(Command::SelfCmd(SelfCommand::List { limit, .. })) = cli.command else {
            panic!("expected self list");
        };
        assert_eq!(limit, Some(5));
    }
}
