//! Rendering results for humans and for machines.
//!
//! Both renderings are produced from the same structured result, so neither can
//! carry information the other lacks. Nothing here computes anything: if a
//! value needs deriving, it belongs in a use case.

use std::io::IsTerminal;

use serde_json::{Value, json};

use crate::app::bump::BumpPlan;
use crate::app::change::{BranchCheck, ChangeSet, OffBranch, Outcome, TagPlan};
use crate::app::check::CheckReport;
use crate::app::push::Pushed;
use crate::app::status::ProjectStatus;
use crate::app::update::{Channel, Listing, Release, UpdateOutcome};
use crate::cli::exit::Exit;
use crate::config::TagStyle;

/// Symbols used to mark pass and fail states.
///
/// Symbols are decorative, so they are replaced by words when output is being
/// captured rather than read by a person.
struct Marks {
    ok: &'static str,
    fail: &'static str,
}

impl Marks {
    fn detect() -> Self {
        if std::io::stdout().is_terminal() {
            Self {
                ok: "✓", fail: "✗"
            }
        } else {
            Self {
                ok: "OK  ",
                fail: "FAIL",
            }
        }
    }
}

/// Renders the outcome of a `check`.
pub fn check(report: &CheckReport, json: bool) {
    if json {
        let files: Vec<Value> = report
            .files
            .iter()
            .map(|f| {
                json!({
                    "path": f.path,
                    "version": f.version.to_string(),
                    "matches": f.version == report.expected,
                })
            })
            .collect();

        print(&json!({
            "command": "check",
            "ok": report.is_satisfied(),
            "expected": report.expected.to_string(),
            "files": files,
        }));
        return;
    }

    let marks = Marks::detect();

    if report.is_satisfied() {
        let count = report.files.len();
        let subject = if count == 1 {
            "file matches"
        } else {
            "files match"
        };
        println!("{} {count} {subject} {}", marks.ok, report.expected);
        return;
    }

    let width = report
        .files
        .iter()
        .map(|f| f.path.len())
        .max()
        .unwrap_or_default();

    eprintln!(
        "{} version mismatch: expected {}",
        marks.fail, report.expected
    );
    eprintln!();
    for file in &report.files {
        let mark = if file.version == report.expected {
            marks.ok
        } else {
            marks.fail
        };
        eprintln!("  {mark}  {:<width$}  {}", file.path, file.version);
    }
    eprintln!();
}

/// Warns that a release is proceeding from a branch the configuration excludes.
///
/// Printed even under `--json`, because it goes to stderr and leaves the
/// machine-readable result on stdout intact. A waived guard that says nothing
/// is a guard nobody notices they turned off.
///
/// This is not the notice `DESIGN.md` decided against. That one announced which
/// source a setting came from, with nothing to do about it. This one reports a
/// hazard the caller can still act on, in the window where acting is cheap.
pub fn off_branch(off: &OffBranch, check: BranchCheck) {
    eprintln!("{}", off.describe(check));
}

/// Renders the outcome of a `status`.
pub fn status(projects: &[ProjectStatus], json: bool) {
    if json {
        let rendered: Vec<Value> = projects
            .iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "in_sync": p.is_in_sync(),
                    "version": p.agreed_version().map(ToString::to_string),
                    "files": p.files.iter().map(|f| json!({
                        "path": f.path,
                        "version": f.version.to_string(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        print(&json!({
            "command": "status",
            "ok": projects.iter().all(ProjectStatus::is_in_sync),
            "projects": rendered,
        }));
        return;
    }

    let marks = Marks::detect();

    for (i, project) in projects.iter().enumerate() {
        // A blank line detaches a file listing from the row above it, so it is
        // earned when either this project or the previous one prints files.
        // Between rows that are a single line each it would only add gaps.
        let after_listing = i > 0 && !projects[i - 1].is_in_sync();
        if after_listing || (i > 0 && !project.is_in_sync()) {
            println!();
        }

        let label = project.name.as_deref().unwrap_or("(unnamed project)");
        match project.agreed_version() {
            Some(version) => println!("{}  {label}  {version}", marks.ok),
            None => println!("{}  {label}  files disagree", marks.fail),
        }

        // Listing every file is noise when they agree; when they do not, the
        // disagreement is the whole point of the report.
        if !project.is_in_sync() {
            let width = project
                .files
                .iter()
                .map(|f| f.path.len())
                .max()
                .unwrap_or_default();
            for file in &project.files {
                println!("      {:<width$}  {}", file.path, file.version);
            }
        }
    }
}

/// Renders a plan as a block of text for a confirmation prompt.
#[must_use]
pub fn summary(plan: &BumpPlan) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out);
    if let Some(project) = plan.changes.project.as_deref() {
        let _ = writeln!(out, "  Project:  {project}");
    }
    let _ = writeln!(
        out,
        "  Bumping:  {}  ->  {}",
        plan.from, plan.changes.target
    );

    for file in &plan.changes.files {
        let _ = writeln!(out, "  File:     {}", file.path);
    }
    if let Some(message) = plan.changes.git.commit.as_deref() {
        let _ = writeln!(out, "  Commit:   {message}");
    }
    if let Some(tag) = plan.changes.git.tag.as_ref() {
        let _ = writeln!(out, "  Tag:      {}{}", tag.name, tag_note(tag));
    }
    let _ = writeln!(
        out,
        "  Push:     {}",
        if plan.changes.git.push { "yes" } else { "no" }
    );

    out
}

/// Renders a change that will not be carried out.
pub fn plan(changes: &ChangeSet, json: bool) {
    if json {
        print(&json!({
            "command": "plan",
            "ok": true,
            "dry_run": true,
            "project": changes.project,
            "current": changes.common_origin().map(ToString::to_string),
            "next": changes.target.to_string(),
            "changes": changes_json(changes),
            "git": git_json(changes),
        }));
        return;
    }

    match changes.common_origin() {
        Some(origin) => println!("{origin} -> {}  (dry run)", changes.target),
        // Files disagree, so there is no single version being moved from; each
        // file's own is shown beside it below.
        None => println!("-> {}  (dry run)", changes.target),
    }
    println!();
    for file in &changes.files {
        if changes.common_origin().is_some() {
            println!("  would write  {}", file.path);
        } else {
            println!("  would write  {}  ({})", file.path, file.from);
        }
    }
    describe_git_plan(changes);
    println!();
    println!("Nothing was written.");
}

/// Renders a run that found nothing to do.
///
/// Reported as success rather than as an error: the files record the version
/// that was asked for, which is the outcome the caller wanted.
pub fn unchanged(changes: &ChangeSet, json: bool) {
    if json {
        print(&json!({
            "command": "set",
            "ok": true,
            "changed": false,
            "project": changes.project,
            "version": changes.target.to_string(),
            "changes": [],
        }));
        return;
    }

    let marks = Marks::detect();
    let count = changes.files.len();
    let subject = if count == 1 { "file" } else { "files" };
    println!(
        "{} {count} {subject} already record {}; nothing to do",
        marks.ok, changes.target
    );
}

/// Renders a completed change.
pub fn applied(changes: &ChangeSet, outcome: &Outcome, json: bool) {
    if json {
        print(&json!({
            "command": "bump",
            "ok": outcome.push_error.is_none(),
            "project": changes.project,
            "previous": changes.common_origin().map(ToString::to_string),
            "version": changes.target.to_string(),
            "changes": changes_json(changes),
            "git": {
                "committed": outcome.committed,
                "commit_message": changes.git.commit,
                "tagged": outcome.tagged,
                "tag": changes.git.tag.as_ref().map(|t| &t.name),
                "tag_style": changes.git.tag.as_ref().map(|t| t.style().as_str()),
                "pushed": outcome.pushed,
                "push_error": outcome.push_error,
            },
        }));
        return;
    }

    let marks = Marks::detect();

    match changes.common_origin() {
        Some(origin) => println!("{} {origin} -> {}", marks.ok, changes.target),
        None => println!("{} set to {}", marks.ok, changes.target),
    }
    for path in &outcome.written {
        println!("  {path}");
    }

    if let Some(message) = changes.git.commit.as_deref().filter(|_| outcome.committed) {
        println!("{} committed  {message}", marks.ok);
    }
    if let Some(tag) = changes.git.tag.as_ref().filter(|_| outcome.tagged) {
        println!("{} tagged     {}{}", marks.ok, tag.name, tag_note(tag));
    }

    if let Some(detail) = outcome.push_error.as_deref() {
        // The commit and tag survived; only the push did not. Saying what
        // remains to be done is more useful than reporting a failed run.
        eprintln!();
        eprintln!("{} push failed: {detail}", marks.fail);
        eprintln!();
        eprintln!("Everything else succeeded. To finish:");
        eprintln!(
            "  {}",
            push_command(changes.git.tag.as_ref().map(|t| t.name.as_str()))
        );
    } else if outcome.pushed {
        println!("{} pushed", marks.ok);
    } else if outcome.committed {
        println!();
        println!("To push:");
        println!(
            "  {}",
            push_command(changes.git.tag.as_ref().map(|t| t.name.as_str()))
        );
    }
}

/// Names how a tag will be written, when that is not the default.
///
/// Signing can fail on a machine without a key, and a lightweight tag is
/// weaker than what a release usually wants — both are worth seeing before
/// they happen, and neither needs saying when the tag is an ordinary
/// annotated one.
fn tag_note(tag: &TagPlan) -> &'static str {
    match tag.style() {
        TagStyle::Annotated => "",
        TagStyle::Lightweight => "  (lightweight)",
        TagStyle::Signed => "  (signed)",
    }
}

/// The command that finishes a run which stopped short of pushing.
///
/// `vump push` where a tag was made: it derives the same single ref from the
/// files and the pattern, so the suggestion cannot name the wrong one, and it
/// is shorter than the two git commands it replaces. Deliberately not
/// `--follow-tags`, which would carry every other reachable annotated tag.
fn push_command(tag: Option<&str>) -> String {
    match tag {
        Some(_) => "vump push".to_owned(),
        None => "git push".to_owned(),
    }
}

fn describe_git_plan(changes: &ChangeSet) {
    if let Some(message) = changes.git.commit.as_deref() {
        println!("  would commit  {message}");
    }
    if let Some(tag) = changes.git.tag.as_ref() {
        println!("  would tag     {}{}", tag.name, tag_note(tag));
    }
    if changes.git.push {
        println!("  would push");
    }
}

fn changes_json(changes: &ChangeSet) -> Vec<Value> {
    changes
        .files
        .iter()
        .map(|f| {
            json!({
                "path": f.path,
                "from": f.from.to_string(),
                "to": changes.target.to_string(),
            })
        })
        .collect()
}

fn git_json(changes: &ChangeSet) -> Value {
    json!({
        "commit_message": changes.git.commit,
        "tag": changes.git.tag.as_ref().map(|t| &t.name),
        "tag_style": changes.git.tag.as_ref().map(|t| t.style().as_str()),
        "push": changes.git.push,
    })
}

/// Renders the outcome of a `self update` or `self status`.
pub fn update(outcome: &UpdateOutcome, json: bool) {
    if json {
        let (state, current, other) = match outcome {
            UpdateOutcome::UpToDate { current } => ("up_to_date", current, None),
            UpdateOutcome::Available { current, latest } => ("available", current, Some(latest)),
            UpdateOutcome::Installed {
                previous,
                installed,
            } => ("installed", previous, Some(installed)),
            UpdateOutcome::Ahead { current, latest } => ("ahead", current, Some(latest)),
            UpdateOutcome::NoneAvailable { current, .. } => ("none_available", current, None),
        };

        let channel = match outcome {
            UpdateOutcome::NoneAvailable { channel, .. } => Some(channel.to_string()),
            _ => None,
        };

        print(&json!({
            "command": "self",
            "ok": true,
            "state": state,
            "current": current.to_string(),
            "latest": other.map(ToString::to_string),
            "channel": channel,
        }));
        return;
    }

    let marks = Marks::detect();

    match outcome {
        UpdateOutcome::UpToDate { current } => {
            println!("{} {current} is the newest release", marks.ok);
        }
        UpdateOutcome::Available { current, latest } => {
            println!("{latest} is available; running {current}.");
            println!("Run `vump self update` to install it.");
        }
        UpdateOutcome::Installed {
            previous,
            installed,
        } => {
            // Naming an older version explicitly is a rollback, and calling it
            // an update would misdescribe what just happened.
            let verb = if installed < previous {
                "rolled back"
            } else {
                "updated"
            };
            println!("{} {verb} {previous} -> {installed}", marks.ok);
        }
        UpdateOutcome::Ahead { current, latest } => {
            println!("{current} is newer than the newest release ({latest}); nothing to do.");
        }
        UpdateOutcome::NoneAvailable { current, channel } => {
            println!("no {channel} release is published; running {current}.");
            if *channel == Channel::Stable {
                println!("Pass --channel rc, beta, or alpha to consider pre-releases.");
            }
        }
    }
}

/// Renders the list of published releases, marking the running one.
pub fn releases(listing: &Listing, limit: Option<usize>, json: bool) {
    let shown: Vec<&Release> = listing
        .releases
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .collect();

    if json {
        print(&json!({
            "command": "self list",
            "ok": true,
            "current": listing.current.to_string(),
            "releases": shown.iter().map(|r| json!({
                "version": r.version.to_string(),
                "tag": r.tag,
                "current": r.version == listing.current,
            })).collect::<Vec<_>>(),
        }));
        return;
    }

    if shown.is_empty() {
        println!("no releases match.");
        return;
    }

    let width = shown
        .iter()
        .map(|r| r.version.to_string().len())
        .max()
        .unwrap_or_default();

    for release in shown {
        let marker = if release.version == listing.current {
            "*"
        } else {
            " "
        };
        println!("{marker} {:<width$}", release.version.to_string());
    }

    // The running version may be a development build that was never published,
    // in which case nothing above is marked and saying so avoids confusion.
    if !listing
        .releases
        .iter()
        .any(|r| r.version == listing.current)
    {
        println!();
        println!("running {} (not a published release)", listing.current);
    }
}

/// Renders the outcome of an `init`.
pub fn init(files: &[String], json: bool) {
    if json {
        print(&json!({
            "command": "init",
            "ok": true,
            "config": crate::config::FILE_NAME,
            "files": files,
        }));
        return;
    }

    let marks = Marks::detect();
    println!("{} wrote {} tracking:", marks.ok, crate::config::FILE_NAME);
    for file in files {
        println!("  {file}");
    }
    println!();
    println!("Review it, then run `vump` to bump.");
}

/// Renders a failure.
pub fn error(message: &str, exit: Exit, json: bool) {
    if json {
        print(&json!({
            "ok": false,
            "error": {
                "kind": exit.as_str(),
                "message": message,
            },
        }));
    } else {
        eprintln!("error: {message}");
    }
}

fn print(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        // Serializing a value built from `json!` cannot fail in practice; if it
        // somehow does, say so rather than exiting silently.
        Err(e) => eprintln!("error: cannot render JSON output: {e}"),
    }
}

/// Reports the commit and tag a push carried, or would carry.
pub fn pushed(pushed: &Pushed, dry_run: bool, json: bool) {
    if json {
        print(&serde_json::json!({
            "action": if dry_run { "push_plan" } else { "pushed" },
            "version": pushed.version.to_string(),
            "tag": pushed.tag,
        }));
        return;
    }

    if dry_run {
        println!("DRY  would push HEAD and {}", pushed.tag);
        println!("\nNothing was pushed.");
    } else {
        println!("OK   pushed HEAD and {}", pushed.tag);
    }
}
