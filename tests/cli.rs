//! End-to-end tests driving the compiled binary.
//!
//! This layer owns the two contracts that cannot be proven anywhere else: that
//! naming a subcommand never results in a prompt, and that each failure mode
//! exits with its documented code. Both are observable only from outside the
//! process.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

/// A scratch repository to run vump against.
struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: TempDir::new().expect("cannot create temp dir"),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(self, name: &str, contents: &str) -> Self {
        let target = self.dir.path().join(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("cannot create parent directory");
        }
        std::fs::write(target, contents).expect("cannot write fixture file");
        self
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(name)).expect("cannot read fixture file")
    }

    /// Runs vump with stdin closed.
    ///
    /// Closed stdin is the point: a command that tried to prompt would have no
    /// terminal to prompt on, so any command that completes here is proven not
    /// to depend on interactive input.
    fn run(&self, args: &[&str]) -> Run {
        let output = Command::new(env!("CARGO_BIN_EXE_vump"))
            .args(args)
            .current_dir(self.dir.path())
            .stdin(Stdio::null())
            .output()
            .expect("cannot run vump");
        Run::from(output)
    }

    /// Runs vump from a subdirectory, with stdin closed.
    ///
    /// Configuration is discovered by searching upward, so where vump is run
    /// from is part of its behavior and has to be expressible in a test.
    fn run_in(&self, dir: &str, args: &[&str]) -> Run {
        let output = Command::new(env!("CARGO_BIN_EXE_vump"))
            .args(args)
            .current_dir(self.dir.path().join(dir))
            .stdin(Stdio::null())
            .output()
            .expect("cannot run vump");
        Run::from(output)
    }

    /// Runs git in a subdirectory of the fixture.
    fn git_in(&self, dir: &str, args: &[&str]) -> &Self {
        let status = Command::new("git")
            .args(args)
            .current_dir(self.dir.path().join(dir))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("cannot run git");
        assert!(status.success(), "git {args:?} failed");
        self
    }

    /// Moves the repository onto `name`, creating it.
    ///
    /// Named explicitly rather than relying on whatever `git init` chose: the
    /// default branch depends on the machine's git configuration, and a test
    /// about branch names cannot be at the mercy of one.
    fn branch(&self, name: &str) -> &Self {
        self.git(&["checkout", "-q", "-b", name])
    }

    /// Leaves the repository on no branch at all.
    fn detach(&self) -> &Self {
        self.git(&["checkout", "-q", "--detach"])
    }

    /// Tags present in the fixture repository, in git's own order.
    fn tags(&self) -> Vec<String> {
        let out = Command::new("git")
            .args(["tag"])
            .current_dir(self.dir.path())
            .output()
            .expect("cannot list tags");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    /// Runs git and returns its trimmed stdout.
    fn git_output(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("cannot run git");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// Paths git reports as changed, one per line.
    fn porcelain(&self) -> String {
        let out = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(self.dir.path())
            .output()
            .expect("cannot read git status");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn git(&self, args: &[&str]) -> &Self {
        let status = Command::new("git")
            .args(args)
            .current_dir(self.dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("cannot run git");
        assert!(status.success(), "git {args:?} failed");
        self
    }

    /// Creates a git repository with an initial commit.
    fn with_git(self) -> Self {
        self.git(&["init", "-q", "."])
            .git(&["config", "user.email", "test@example.com"])
            .git(&["config", "user.name", "Test"])
            .git(&["config", "commit.gpgsign", "false"])
            .git(&["add", "-A"])
            .git(&["commit", "-qm", "initial"]);
        self
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl From<Output> for Run {
    fn from(output: Output) -> Self {
        Self {
            code: output.status.code().expect("process was terminated"),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

impl Run {
    fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}", self.stdout))
    }
}

const SINGLE: &str = "files = [\"VERSION\"]\n";

// ─── Exit codes ──────────────────────────────────────────────────────────────

#[test]
fn check_exits_zero_when_versions_match() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    let run = fx.run(&["check", "1.2.3"]);
    assert_eq!(run.code, 0, "{}", run.output());
}

#[test]
fn check_accepts_a_v_prefixed_tag() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "0.2.0-alpha.1\n");

    assert_eq!(fx.run(&["check", "v0.2.0-alpha.1"]).code, 0);
}

#[test]
fn check_exits_four_on_mismatch() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    let run = fx.run(&["check", "9.9.9"]);
    assert_eq!(run.code, 4, "{}", run.output());
    assert!(run.stderr.contains("9.9.9"));
}

#[test]
fn a_malformed_version_argument_exits_two() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    assert_eq!(fx.run(&["check", "not-a-version"]).code, 2);
}

#[test]
fn missing_configuration_exits_three() {
    let fx = Fixture::new();
    let run = fx.run(&["status"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("vump.toml"));
}

#[test]
fn disagreeing_files_exit_five() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\", \"Cargo.toml\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("Cargo.toml", "[package]\nversion = \"1.0.0\"\n");

    let run = fx.run(&["patch", "--through", "none"]);
    assert_eq!(run.code, 5, "{}", run.output());
}

#[test]
fn a_dirty_tree_exits_six() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n")
        .with_git()
        .write("stray.txt", "uncommitted");

    let run = fx.run(&["patch", "--through", "commit"]);
    assert_eq!(run.code, 6, "{}", run.output());
    // Refusing must leave the version untouched.
    assert_eq!(fx.read("VERSION"), "1.2.3\n");
}

#[test]
fn an_impossible_transition_exits_seven() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3-rc.1\n");

    let run = fx.run(&["patch", "--through", "none"]);
    assert_eq!(run.code, 7, "{}", run.output());
    // The message must point at the command that resolves the ambiguity.
    assert!(run.stderr.contains("release"), "{}", run.stderr);
}

#[test]
fn starting_a_pre_release_without_toward_exits_seven() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    let run = fx.run(&["alpha", "--through", "none"]);
    assert_eq!(run.code, 7, "{}", run.output());
    assert!(run.stderr.contains("--toward"), "{}", run.stderr);
}

// ─── The non-interactive contract ────────────────────────────────────────────

#[test]
fn no_subcommand_ever_waits_for_input() {
    // Each of these would have needed a question from the user under a design
    // that falls back to prompting. With stdin closed, any of them hanging or
    // consuming input would fail this test.
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\", \"Cargo.toml\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("Cargo.toml", "[package]\nversion = \"1.0.0\"\n");

    // Files disagree: resolved by erroring, not by asking which to trust.
    assert_eq!(fx.run(&["patch", "--through", "none"]).code, 5);

    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    // Pre-release from stable: resolved by requiring a flag, not by asking.
    assert_eq!(fx.run(&["beta", "--through", "none"]).code, 7);

    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3-beta.2\n");

    // Stable bump on a pre-release: resolved by erroring, not by asking.
    assert_eq!(fx.run(&["minor", "--through", "none"]).code, 7);
}

#[test]
fn the_interactive_path_refuses_without_a_terminal_instead_of_hanging() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    // Bare vump is the interactive entry point. With no terminal attached it
    // must say so and stop, never block waiting for input that cannot arrive.
    let run = fx.run(&[]);
    assert_eq!(run.code, 2, "{}", run.output());
    assert!(run.stderr.contains("terminal"), "{}", run.stderr);
    assert_eq!(fx.read("VERSION"), "1.2.3\n");
}

// ─── Bumping ─────────────────────────────────────────────────────────────────

#[test]
fn a_bump_writes_every_tracked_file() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\", \"ui/package.json\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("ui/package.json", "{\n  \"version\": \"1.2.3\"\n}\n");

    let run = fx.run(&["minor", "--through", "none"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.read("VERSION"), "1.3.0\n");
    assert!(fx.read("ui/package.json").contains("\"1.3.0\""));
}

#[test]
fn dry_run_writes_nothing() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    let run = fx.run(&["major", "--dry-run", "--through", "none"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.read("VERSION"), "1.2.3\n");
    assert!(run.stdout.contains("2.0.0"));
}

#[test]
fn a_pre_release_sequence_advances_and_finalizes() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    assert_eq!(
        fx.run(&["alpha", "--toward", "minor", "--through", "none"])
            .code,
        0
    );
    assert_eq!(fx.read("VERSION"), "1.3.0-alpha.0\n");

    assert_eq!(fx.run(&["alpha", "--through", "none"]).code, 0);
    assert_eq!(fx.read("VERSION"), "1.3.0-alpha.1\n");

    assert_eq!(fx.run(&["rc", "--through", "none"]).code, 0);
    assert_eq!(fx.read("VERSION"), "1.3.0-rc.0\n");

    assert_eq!(fx.run(&["release", "--through", "none"]).code, 0);
    assert_eq!(fx.read("VERSION"), "1.3.0\n");
}

#[test]
fn moving_to_a_less_mature_channel_is_refused() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3-rc.2\n");

    let run = fx.run(&["alpha", "--through", "none"]);
    assert_eq!(run.code, 7, "{}", run.output());
    assert_eq!(fx.read("VERSION"), "1.2.3-rc.2\n");
}

#[test]
fn a_bump_commits_and_tags_when_asked() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n")
        .with_git();

    let run = fx.run(&["patch", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());

    assert_eq!(fx.tags(), ["v1.2.4"]);
}

#[test]
fn configuration_alone_drives_git_actions() {
    // No git flags are passed: settings in vump.toml are decisions already
    // made, and must be acted upon rather than ignored.
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n[git]\nthrough = \"tag\"\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();

    assert_eq!(fx.run(&["patch"]).code, 0);

    assert_eq!(fx.tags(), ["v1.2.4"]);
}

#[test]
fn a_configured_step_can_be_lowered_for_one_run() {
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n[git]\nthrough = \"tag\"\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();

    assert_eq!(fx.run(&["patch", "--through", "none"]).code, 0);
    assert_eq!(fx.read("VERSION"), "1.2.4\n");

    assert_eq!(fx.tags(), Vec::<String>::new());
}

#[test]
fn a_configured_step_can_be_raised_for_one_run() {
    // The override reads in both directions. Raising it is the case a flag
    // that only ever added could already express; lowering it was not.
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n[git]\nthrough = \"commit\"\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();

    assert_eq!(fx.run(&["patch", "--through", "tag"]).code, 0);

    assert_eq!(fx.tags(), ["v1.2.4"]);
}

#[test]
fn a_repository_that_declares_no_git_work_gets_none() {
    // `through = "none"` is a decision, and the sandbox depends on it: those
    // projects sit inside vump's own repository, so a commit made there lands
    // in it. Silence used to be indistinguishable from this, which left no way
    // to say "never".
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n[git]\nthrough = \"none\"\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();

    assert_eq!(fx.run(&["patch"]).code, 0);
    assert_eq!(fx.read("VERSION"), "1.2.4\n");

    assert_eq!(fx.tags(), Vec::<String>::new());
}

/// A repository whose own configuration is shadowed by one in `inner/`.
fn nested() -> Fixture {
    Fixture::new()
        .write("vump.toml", "files = [\"VERSION\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("inner/vump.toml", "files = [\"VERSION\"]\n")
        .write("inner/VERSION", "9.9.9\n")
        .with_git()
}

#[test]
fn a_nested_configuration_may_not_be_written_from_without_acknowledgement() {
    // The accident this exists for: `vump patch --through push` run inside
    // sandbox/npm/single-project tagged vump's own repository. Discovery stops
    // at the nearest vump.toml, but the repository being written to is the one
    // the outer configuration describes.
    let fx = nested();

    let run = fx.run_in("inner", &["patch", "--through", "tag"]);
    assert_eq!(run.code, 3, "{}", run.output());

    // The message has to say where the caller actually is, or it reads as an
    // obstacle to get past rather than as a wrong turn. The separator is the
    // platform's: this is a location on disk, not a path declared in vump.toml.
    let tail = Path::new("inner").join("vump.toml");
    assert!(
        run.stderr.contains(&tail.display().to_string()),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("[[project]]"), "{}", run.stderr);

    // Refusing must leave everything as it was.
    assert_eq!(fx.read("inner/VERSION"), "9.9.9\n");
    assert_eq!(fx.tags(), Vec::<String>::new());
}

#[test]
fn a_nested_configuration_is_refused_even_when_no_git_work_is_asked_for() {
    // Writing is the line, not committing. Refusing only the runs that reach
    // git would warn about the same layout sometimes and not others, and the
    // files a bump writes sit in the outer repository's tree either way.
    let fx = nested();

    let run = fx.run_in("inner", &["patch", "--through", "none"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert_eq!(fx.read("inner/VERSION"), "9.9.9\n");
}

#[test]
fn a_nested_dry_run_is_refused_rather_than_reporting_a_plan() {
    // --dry-run reports what a real run would do. A plan for a run that would
    // be refused is not that.
    let fx = nested();

    let run = fx.run_in("inner", &["patch", "--through", "none", "--dry-run"]);
    assert_eq!(run.code, 3, "{}", run.output());
}

#[test]
fn even_reading_a_nested_configuration_is_refused() {
    // Writing is not the hazard; using the wrong configuration at all is.
    // `check` is the sharpest case: it answers whether a version matches, and
    // a nested project whose version happens to coincide would answer yes
    // about the wrong project — a confident false pass from the one command
    // whose whole job is catching a version that lies.
    let fx = nested();

    assert_eq!(fx.run_in("inner", &["status"]).code, 3);
    assert_eq!(fx.run_in("inner", &["check", "9.9.9"]).code, 3);

    // Refusing is where a nested layout gets named, so the message carries the
    // file that answered rather than leaving it to be worked out.
    let run = fx.run_in("inner", &["status"]);
    let tail = Path::new("inner").join("vump.toml");
    assert!(
        run.stderr.contains(&tail.display().to_string()),
        "{}",
        run.stderr
    );
}

#[test]
fn init_refuses_to_create_a_nested_configuration() {
    // The place it matters most: this is where the arrangement every other
    // command has to refuse would come into being.
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("inner/VERSION", "9.9.9\n")
        .with_git();

    let run = fx.run_in("inner", &["init"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("[[project]]"), "{}", run.stderr);
    assert!(
        !fx.path().join("inner/vump.toml").exists(),
        "refusing must not leave a configuration behind"
    );
}

#[test]
fn init_at_the_repository_root_is_unaffected() {
    let fx = Fixture::new().write("VERSION", "1.2.3\n").with_git();

    assert_eq!(fx.run(&["init"]).code, 0);
    assert!(fx.path().join("vump.toml").exists());
}

#[test]
fn a_guided_run_refuses_before_asking_anything() {
    // Run with stdin closed: reaching any prompt exits 2, because there is no
    // terminal to ask on. Exiting 3 instead is what proves the refusal happens
    // before the first question rather than after every answer is in.
    let fx = nested();

    let run = fx.run_in("inner", &[]);
    assert_eq!(run.code, 3, "{}", run.output());
    let tail = Path::new("inner").join("vump.toml");
    assert!(
        run.stderr.contains(&tail.display().to_string()),
        "{}",
        run.stderr
    );
}

#[test]
fn an_acknowledged_nested_configuration_proceeds() {
    let fx = nested();

    let run = fx.run_in("inner", &["patch", "--through", "tag", "--allow-nested"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.tags(), ["v9.9.10"]);
}

#[test]
fn a_configuration_at_the_repository_root_is_not_nested() {
    // A vump.toml above the repository root describes a different repository
    // and says nothing about this one, so the search must stop at the root
    // rather than walking to the filesystem's.
    let fx = Fixture::new()
        .write("outer-vump-decoy/vump.toml", "files = [\"VERSION\"]\n")
        .write("repo/vump.toml", "files = [\"VERSION\"]\n")
        .write("repo/VERSION", "1.2.3\n");

    fx.git_in("repo", &["init", "-q", "."]);
    fx.git_in("repo", &["config", "user.email", "test@example.com"]);
    fx.git_in("repo", &["config", "user.name", "Test"]);
    fx.git_in("repo", &["config", "commit.gpgsign", "false"]);
    fx.git_in("repo", &["add", "-A"]);
    fx.git_in("repo", &["commit", "-qm", "initial"]);

    let run = fx.run_in("repo", &["patch", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());
}

#[test]
fn a_tag_style_can_be_overridden_without_editing_the_repository() {
    // A signing key that is temporarily out of reach must not require a commit
    // to a tracked file to work around.
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n[git]\nthrough = \"tag\"\ntag_style = \"signed\"\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();

    let run = fx.run(&["patch", "--tag-style", "lightweight"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(fx.tags(), ["v1.2.4"]);
}

// ─── Setting an exact version ────────────────────────────────────────────────

#[test]
fn set_repairs_files_that_a_bump_refuses() {
    // The dead end this command exists for: `patch` exits 5 here, and the only
    // other repair was editing by hand.
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\", \"Cargo.toml\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("Cargo.toml", "[package]\nversion = \"0.9.0\"\n");

    assert_eq!(fx.run(&["patch", "--through", "none"]).code, 5);

    let run = fx.run(&["set", "2.0.0", "--through", "none"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.read("VERSION"), "2.0.0\n");
    assert!(fx.read("Cargo.toml").contains("2.0.0"));

    // Repaired, so an ordinary bump works again.
    assert_eq!(fx.run(&["patch", "--through", "none"]).code, 0);
    assert_eq!(fx.read("VERSION"), "2.0.1\n");
}

#[test]
fn set_accepts_a_v_prefix() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.0.0\n");

    assert_eq!(fx.run(&["set", "v2.5.0", "--through", "none"]).code, 0);
    assert_eq!(fx.read("VERSION"), "2.5.0\n");
}

#[test]
fn set_moves_backwards_without_complaint() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "2.0.0\n");

    assert_eq!(fx.run(&["set", "1.0.0", "--through", "none"]).code, 0);
    assert_eq!(fx.read("VERSION"), "1.0.0\n");
}

#[test]
fn setting_the_recorded_version_is_a_no_op_not_a_failure() {
    // Configuration asks for a commit, and git refuses an empty one. The run
    // must report success rather than fail for a reason unrelated to the ask.
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n[git]\nthrough = \"commit\"\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();

    let run = fx.run(&["set", "1.2.3"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("nothing to do"), "{}", run.stdout);
    assert_eq!(fx.tags(), Vec::<String>::new());
}

#[test]
fn set_commits_and_tags_like_a_bump() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.0.0\n")
        .with_git();

    assert_eq!(fx.run(&["set", "4.5.6", "--through", "tag"]).code, 0);
    assert_eq!(fx.tags(), ["v4.5.6"]);
}

#[test]
fn set_honours_dry_run() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.0.0\n");

    let run = fx.run(&["set", "9.9.9", "--dry-run", "--through", "none"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.read("VERSION"), "1.0.0\n");
    assert!(run.stdout.contains("9.9.9"));
}

#[test]
fn set_reports_each_files_previous_version_as_json() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\", \"Cargo.toml\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("Cargo.toml", "[package]\nversion = \"0.9.0\"\n");

    let value = fx
        .run(&["set", "2.0.0", "--through", "none", "--json"])
        .json();

    assert_eq!(value["version"], "2.0.0");
    // Files disagreed, so there is no single version they moved from.
    assert!(value["previous"].is_null());
    assert_eq!(value["changes"][0]["from"], "1.2.3");
    assert_eq!(value["changes"][1]["from"], "0.9.0");
}

#[test]
fn a_malformed_set_argument_exits_two() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.0.0\n");

    assert_eq!(
        fx.run(&["set", "not-a-version", "--through", "none"]).code,
        2
    );
    assert_eq!(fx.read("VERSION"), "1.0.0\n");
}

// ─── Initialization ──────────────────────────────────────────────────────────

#[test]
fn init_writes_a_config_the_next_command_can_use() {
    let fx = Fixture::new()
        .write("VERSION", "0.1.0\n")
        .write("apps/web/package.json", "{\"version\":\"0.1.0\"}")
        .write("node_modules/dep/package.json", "{\"version\":\"9.9.9\"}");

    let run = fx.run(&["init"]);
    assert_eq!(run.code, 0, "{}", run.output());

    let config = fx.read("vump.toml");
    assert!(config.contains("VERSION"));
    assert!(config.contains("apps/web/package.json"));
    assert!(
        !config.contains("node_modules"),
        "a dependency's manifest is not this project's version:\n{config}"
    );

    // The generated file must be usable immediately, without editing.
    assert_eq!(fx.run(&["check", "0.1.0"]).code, 0);
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let fx = Fixture::new()
        .write("VERSION", "0.1.0\n")
        .write("vump.toml", SINGLE);

    assert_eq!(fx.run(&["init"]).code, 3);
    assert_eq!(fx.run(&["init", "--force"]).code, 0);
}

#[test]
fn init_reports_when_there_is_nothing_to_track() {
    let fx = Fixture::new().write("README.md", "nothing versioned here");
    let run = fx.run(&["init"]);
    assert_eq!(run.code, 3, "{}", run.output());
}

// ─── Multi-project repositories ──────────────────────────────────────────────

const MULTI: &str = "\
[[project]]
name = \"api\"
files = [\"api/Cargo.toml\"]

[[project]]
name = \"web\"
files = [\"web/package.json\"]
";

#[test]
fn projects_are_versioned_independently() {
    let fx = Fixture::new()
        .write("vump.toml", MULTI)
        .write("api/Cargo.toml", "[package]\nversion = \"1.0.0\"\n")
        .write("web/package.json", "{\"version\":\"3.4.5\"}");

    assert_eq!(
        fx.run(&["patch", "--project", "api", "--through", "none"])
            .code,
        0
    );

    assert!(fx.read("api/Cargo.toml").contains("1.0.1"));
    assert!(
        fx.read("web/package.json").contains("3.4.5"),
        "the other project must be untouched"
    );
}

/// A monorepo where each project carries its own tag pattern.
const MULTI_TAGGED: &str = "\
[[project]]
name = \"api\"
files = [\"api/Cargo.toml\"]
tag_pattern = \"api-v{new_version}\"

[[project]]
name = \"web\"
files = [\"web/package.json\"]
tag_pattern = \"web-v{new_version}\"
";

fn tagged_monorepo() -> Fixture {
    Fixture::new()
        .write("vump.toml", MULTI_TAGGED)
        .write("api/Cargo.toml", "[package]\nversion = \"1.0.0\"\n")
        .write("web/package.json", "{\"version\":\"3.4.5\"}")
}

#[test]
fn a_tag_identifies_its_own_project() {
    // This is the CI case: a workflow passes the pushed tag straight through,
    // without knowing or saying which project it belongs to.
    let fx = tagged_monorepo();

    assert_eq!(fx.run(&["check", "api-v1.0.0"]).code, 0);
    assert_eq!(fx.run(&["check", "web-v3.4.5"]).code, 0);

    // The version is compared against the right project, not just any.
    assert_eq!(fx.run(&["check", "api-v3.4.5"]).code, 4);
    assert_eq!(fx.run(&["check", "web-v1.0.0"]).code, 4);
}

#[test]
fn a_bump_tags_with_the_projects_own_pattern() {
    let fx = tagged_monorepo().with_git();

    let run = fx.run(&["patch", "--project", "api", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());

    assert_eq!(fx.tags(), ["api-v1.0.1"]);
}

#[test]
fn independently_versioned_projects_do_not_collide_on_tags() {
    // Both projects reaching the same version is exactly the case a single
    // repository-wide pattern could not express.
    let fx = Fixture::new()
        .write("vump.toml", MULTI_TAGGED)
        .write("api/Cargo.toml", "[package]\nversion = \"2.0.0\"\n")
        .write("web/package.json", "{\"version\":\"2.0.0\"}")
        .with_git();

    assert_eq!(
        fx.run(&["patch", "--project", "api", "--through", "tag"])
            .code,
        0
    );
    assert_eq!(
        fx.run(&["patch", "--project", "web", "--through", "tag"])
            .code,
        0
    );

    let mut listed = fx.tags();
    listed.sort();
    assert_eq!(listed, ["api-v2.0.1", "web-v2.0.1"]);
}

#[test]
fn a_tag_conflicting_with_an_explicit_project_is_refused() {
    let fx = tagged_monorepo();
    let run = fx.run(&["check", "api-v1.0.0", "--project", "web"]);
    assert_eq!(run.code, 2, "{}", run.output());
    assert!(run.stderr.contains("api"), "{}", run.stderr);
}

#[test]
fn projects_sharing_a_tag_pattern_report_the_ambiguity() {
    // Without distinct patterns there is no way to attribute a tag, and
    // guessing would defeat the point of checking it.
    let fx = Fixture::new()
        .write("vump.toml", MULTI)
        .write("api/Cargo.toml", "[package]\nversion = \"1.0.0\"\n")
        .write("web/package.json", "{\"version\":\"3.4.5\"}");

    let run = fx.run(&["check", "v1.0.0"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("tag_pattern"), "{}", run.stderr);
}

#[test]
fn a_single_project_repository_keeps_accepting_bare_versions() {
    // The tag pattern now matches "v1.2.3", but "1.2.3" must keep working.
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    assert_eq!(fx.run(&["check", "1.2.3"]).code, 0);
    assert_eq!(fx.run(&["check", "v1.2.3"]).code, 0);
    assert_eq!(fx.run(&["check", "v9.9.9"]).code, 4);
}

#[test]
fn a_bump_without_a_project_is_refused_when_several_exist() {
    let fx = Fixture::new()
        .write("vump.toml", MULTI)
        .write("api/Cargo.toml", "[package]\nversion = \"1.0.0\"\n")
        .write("web/package.json", "{\"version\":\"3.4.5\"}");

    let run = fx.run(&["patch", "--through", "none"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("--project"), "{}", run.stderr);
}

#[test]
fn configuration_is_found_from_a_subdirectory() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n")
        .write("deep/nested/placeholder", "");

    let output = Command::new(env!("CARGO_BIN_EXE_vump"))
        .args(["check", "1.2.3"])
        .current_dir(fx.path().join("deep").join("nested"))
        .stdin(Stdio::null())
        .output()
        .expect("cannot run vump");

    assert_eq!(Run::from(output).code, 0);
}

// ─── Machine-readable output ─────────────────────────────────────────────────

#[test]
fn check_reports_each_file_as_json() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\", \"Cargo.toml\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("Cargo.toml", "[package]\nversion = \"1.0.0\"\n");

    let run = fx.run(&["check", "1.2.3", "--json"]);
    let value = run.json();

    assert_eq!(value["ok"], false);
    assert_eq!(value["expected"], "1.2.3");
    let files = value["files"].as_array().expect("files must be an array");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["matches"], true);
    assert_eq!(files[1]["matches"], false);
}

#[test]
fn a_bump_reports_the_resulting_version_as_json() {
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    let value = fx.run(&["minor", "--through", "none", "--json"]).json();

    assert_eq!(value["ok"], true);
    assert_eq!(value["previous"], "1.2.3");
    assert_eq!(value["version"], "1.3.0");
    assert_eq!(value["changes"][0]["path"], "VERSION");
}

#[test]
fn failures_are_reported_as_json_too() {
    let fx = Fixture::new().write("vump.toml", SINGLE);

    let run = fx.run(&["check", "1.0.0", "--json"]);
    let value = run.json();

    assert_eq!(value["ok"], false);
    // The error kind mirrors the exit code, so a caller can branch on either.
    assert_eq!(value["error"]["kind"], "config");
    assert_eq!(run.code, 3);
}

// ─── Lock files ──────────────────────────────────────────────────────────────

const CARGO_PROJECT: &str = "files = [\"Cargo.toml\", \"Cargo.lock\"]\n";

fn manifest(version: &str) -> String {
    format!("[package]\nname = \"demo\"\nversion = \"{version}\"\n")
}

/// A lock as Cargo writes one: a fetched dependency, then the local package.
fn lock(version: &str) -> String {
    format!(
        "version = 4\n\n\
         [[package]]\nname = \"dep\"\nversion = \"1.0.0\"\n\
         source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
         [[package]]\nname = \"demo\"\nversion = \"{version}\"\n"
    )
}

#[test]
fn a_bump_moves_the_lock_file_in_the_same_commit() {
    let fx = Fixture::new()
        .write("vump.toml", CARGO_PROJECT)
        .write("Cargo.toml", &manifest("1.2.3"))
        .write("Cargo.lock", &lock("1.2.3"))
        .with_git();

    let run = fx.run(&["patch", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());

    assert!(fx.read("Cargo.toml").contains("version = \"1.2.4\""));
    assert!(
        fx.read("Cargo.lock")
            .contains("name = \"demo\"\nversion = \"1.2.4\"")
    );
    assert_eq!(fx.tags(), ["v1.2.4"]);

    // Nothing is left behind for the user to finish: this is the whole point.
    // A tag pointing at a tree whose lock still holds the old version is what
    // `cargo build --locked` rejects, after the tag is already public.
    assert_eq!(fx.porcelain(), "", "the bump left work undone");
}

#[test]
fn a_bump_leaves_dependency_versions_in_the_lock_alone() {
    let fx = Fixture::new()
        .write("vump.toml", CARGO_PROJECT)
        .write("Cargo.toml", &manifest("1.2.3"))
        .write("Cargo.lock", &lock("1.2.3"));

    assert_eq!(fx.run(&["minor", "--through", "none"]).code, 0);
    assert!(
        fx.read("Cargo.lock")
            .contains("name = \"dep\"\nversion = \"1.0.0\"")
    );
}

#[test]
fn an_undeclared_lock_stops_the_run_before_anything_happens() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"Cargo.toml\"]\n")
        .write("Cargo.toml", &manifest("1.2.3"))
        .write("Cargo.lock", &lock("1.2.3"))
        .with_git();

    let run = fx.run(&["patch", "--through", "tag"]);

    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.output().contains("Cargo.lock"), "{}", run.output());

    // Refused at planning time, so there is no commit or tag to undo.
    assert!(fx.read("Cargo.toml").contains("1.2.3"));
    assert_eq!(fx.tags(), Vec::<String>::new());
    assert_eq!(fx.porcelain(), "");
}

#[test]
fn check_verifies_the_lock_file_too() {
    // The safety net in CI: a tag matching the manifest but not the lock
    // describes a tree that cannot be built from it.
    let fx = Fixture::new()
        .write("vump.toml", CARGO_PROJECT)
        .write("Cargo.toml", &manifest("1.2.3"))
        .write("Cargo.lock", &lock("1.2.2"));

    let run = fx.run(&["check", "1.2.3"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(run.output().contains("Cargo.lock"), "{}", run.output());
}

#[test]
fn set_repairs_a_manifest_and_lock_that_disagree() {
    let fx = Fixture::new()
        .write("vump.toml", CARGO_PROJECT)
        .write("Cargo.toml", &manifest("1.2.3"))
        .write("Cargo.lock", &lock("1.2.2"));

    assert_eq!(fx.run(&["set", "1.2.3", "--through", "none"]).code, 0);
    assert!(
        fx.read("Cargo.lock")
            .contains("name = \"demo\"\nversion = \"1.2.3\"")
    );
    assert_eq!(fx.run(&["check", "1.2.3"]).code, 0);
}

/// npm's own output, shared with the unit tests. See its `testdata/README.md`.
const NPM_LOCK: &str = include_str!("../src/domain/testdata/npm-lock-v3.json");

const NPM_PROJECT: &str = "files = [\"package.json\", \"package-lock.json\"]\n";

#[test]
fn a_bump_moves_the_npm_lock_in_the_same_commit() {
    let fx = Fixture::new()
        .write("vump.toml", NPM_PROJECT)
        .write("package.json", "{\"name\":\"demo\",\"version\":\"1.2.3\"}")
        .write("package-lock.json", NPM_LOCK)
        .with_git();

    let run = fx.run(&["minor", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());

    let lock = fx.read("package-lock.json");
    // Both records move: `npm ci` reads the one under `packages`.
    assert_eq!(lock.matches("\"1.3.0\"").count(), 2, "{lock}");
    assert!(!lock.contains("\"1.2.3\""), "{lock}");
    assert_eq!(fx.tags(), ["v1.3.0"]);
    assert_eq!(fx.porcelain(), "", "the bump left work undone");
}

#[test]
fn a_bump_leaves_npm_dependency_versions_alone() {
    let fx = Fixture::new()
        .write("vump.toml", NPM_PROJECT)
        .write("package.json", "{\"name\":\"demo\",\"version\":\"1.2.3\"}")
        .write("package-lock.json", NPM_LOCK);

    assert_eq!(fx.run(&["patch", "--through", "none"]).code, 0);
    assert!(fx.read("package-lock.json").contains("\"2.1.3\""));
}

#[test]
fn an_undeclared_npm_lock_stops_the_run_before_anything_happens() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"package.json\"]\n")
        .write("package.json", "{\"name\":\"demo\",\"version\":\"1.2.3\"}")
        .write("package-lock.json", NPM_LOCK)
        .with_git();

    let run = fx.run(&["patch", "--through", "tag"]);

    assert_eq!(run.code, 3, "{}", run.output());
    assert!(
        run.output().contains("package-lock.json"),
        "{}",
        run.output()
    );
    assert_eq!(fx.tags(), Vec::<String>::new());
    assert_eq!(fx.porcelain(), "");
}

#[test]
fn check_verifies_the_npm_lock_too() {
    let fx = Fixture::new()
        .write("vump.toml", NPM_PROJECT)
        .write("package.json", "{\"name\":\"demo\",\"version\":\"9.9.9\"}")
        .write("package-lock.json", NPM_LOCK);

    let run = fx.run(&["check", "9.9.9"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(
        run.output().contains("package-lock.json"),
        "{}",
        run.output()
    );
}

#[test]
fn a_yarn_lock_is_never_demanded() {
    // Yarn pins its workspace at a placeholder, so a bump cannot stale it.
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"package.json\"]\n")
        .write("package.json", "{\"name\":\"demo\",\"version\":\"1.2.3\"}")
        .write("yarn.lock", "# yarn lockfile v1\n");

    assert_eq!(fx.run(&["patch", "--through", "none"]).code, 0);
}

/// Cargo's own output for a two-member workspace, shared with the unit tests.
const WORKSPACE_LOCK: &str = include_str!("../src/domain/testdata/cargo-workspace.lock");

/// A workspace whose members are versioned independently, sharing one lock.
fn workspace(files: &str) -> Fixture {
    Fixture::new()
        .write("vump.toml", files)
        .write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n",
        )
        .write(
            "crates/api/Cargo.toml",
            "[package]\nname = \"api\"\nversion = \"1.0.0\"\n",
        )
        .write(
            "crates/web/Cargo.toml",
            "[package]\nname = \"web\"\nversion = \"2.5.0\"\n",
        )
        .write("Cargo.lock", WORKSPACE_LOCK)
}

#[test]
fn a_workspace_member_bumps_only_its_own_entry_in_the_shared_lock() {
    let fx = workspace("files = [\"crates/api/Cargo.toml\", \"Cargo.lock\"]\n");

    let run = fx.run(&["minor", "--through", "none"]);
    assert_eq!(run.code, 0, "{}", run.output());

    let lock = fx.read("Cargo.lock");
    assert!(
        lock.contains("name = \"api\"\nversion = \"1.1.0\""),
        "{lock}"
    );
    // The sibling member is another project's business.
    assert!(
        lock.contains("name = \"web\"\nversion = \"2.5.0\""),
        "{lock}"
    );
    assert!(lock.contains("version = \"1.0.28\""), "{lock}");
}

#[test]
fn each_project_bumps_its_own_member_of_a_shared_lock() {
    let fx = workspace(
        "[[project]]\nname = \"api\"\nfiles = [\"crates/api/Cargo.toml\", \"Cargo.lock\"]\n\n\
         [[project]]\nname = \"web\"\nfiles = [\"crates/web/Cargo.toml\", \"Cargo.lock\"]\n",
    );

    assert_eq!(
        fx.run(&["patch", "--project", "web", "--through", "none"])
            .code,
        0
    );

    let lock = fx.read("Cargo.lock");
    assert!(
        lock.contains("name = \"web\"\nversion = \"2.5.1\""),
        "{lock}"
    );
    assert!(
        lock.contains("name = \"api\"\nversion = \"1.0.0\""),
        "{lock}"
    );
}

#[test]
fn an_undeclared_workspace_lock_stops_the_run() {
    // The lock sits at the workspace root, directories above the manifest.
    let fx = workspace("files = [\"crates/api/Cargo.toml\"]\n");

    let run = fx.run(&["patch", "--through", "none"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.output().contains("Cargo.lock"), "{}", run.output());
    assert!(fx.read("crates/api/Cargo.toml").contains("1.0.0"));
}

#[test]
fn init_skips_files_it_could_not_keep_in_step() {
    // Members held at one version, which is the shape init can describe: a
    // single project tracking every manifest and the lock they share.
    let fx = Fixture::new()
        .write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n",
        )
        .write(
            "crates/api/Cargo.toml",
            "[package]\nname = \"api\"\nversion = \"1.0.0\"\n",
        )
        .write(
            "crates/web/Cargo.toml",
            "[package]\nname = \"web\"\nversion = \"1.0.0\"\n",
        )
        .write(
            "Cargo.lock",
            &WORKSPACE_LOCK.replace("version = \"2.5.0\"", "version = \"1.0.0\""),
        );

    assert_eq!(fx.run(&["init"]).code, 0);
    let config = fx.read("vump.toml");

    // The virtual workspace manifest declares no version of its own, so
    // tracking it would produce a configuration that fails on every command.
    assert!(!config.contains("\n    \"Cargo.toml\""), "{config}");
    assert!(config.contains("crates/api/Cargo.toml"), "{config}");
    assert!(config.contains("Cargo.lock"), "{config}");

    // And the configuration it wrote works.
    assert_eq!(fx.run(&["check", "1.0.0"]).code, 0);
}

#[test]
fn init_declares_the_lock_beside_the_manifest() {
    let fx = Fixture::new()
        .write("Cargo.toml", &manifest("1.2.3"))
        .write("Cargo.lock", &lock("1.2.3"));

    assert_eq!(fx.run(&["init"]).code, 0);
    let config = fx.read("vump.toml");
    assert!(config.contains("Cargo.toml"), "{config}");
    assert!(config.contains("Cargo.lock"), "{config}");
}

// ─── Tag objects ─────────────────────────────────────────────────────────────

/// A fixture whose bump will tag.
fn taggable(git: &str) -> Fixture {
    Fixture::new()
        .write(
            "vump.toml",
            &format!("files = [\"VERSION\"]\n\n[git]\n{git}"),
        )
        .write("VERSION", "1.2.3\n")
        .with_git()
}

#[test]
fn tags_are_annotated_by_default() {
    let fx = taggable("through = \"tag\"\n");
    assert_eq!(fx.run(&["patch"]).code, 0);

    // git's own answer: an annotated tag is a `tag` object, a lightweight one
    // points straight at the commit.
    assert_eq!(fx.git_output(&["cat-file", "-t", "v1.2.4"]), "tag");
    assert_eq!(
        fx.git_output(&["tag", "-l", "--format=%(contents)", "v1.2.4"]),
        "Release 1.2.4"
    );
}

#[test]
fn a_lightweight_tag_is_available_on_request() {
    let fx = taggable("through = \"tag\"\ntag_style = \"lightweight\"\n");
    assert_eq!(fx.run(&["patch"]).code, 0);

    assert_eq!(fx.git_output(&["cat-file", "-t", "v1.2.4"]), "commit");
}

#[test]
fn a_tag_message_is_configurable() {
    let fx = taggable("through = \"tag\"\ntag_message = \"cut {new_version}\"\n");
    assert_eq!(fx.run(&["patch"]).code, 0);

    assert_eq!(
        fx.git_output(&["tag", "-l", "--format=%(contents)", "v1.2.4"]),
        "cut 1.2.4"
    );
}

#[test]
fn an_unusual_tag_style_is_named_before_it_happens() {
    // Signing can fail on a machine without a key, and a lightweight tag is
    // weaker than a release usually wants; both are worth seeing in a dry run.
    let fx = taggable("through = \"tag\"\ntag_style = \"lightweight\"\n");

    let run = fx.run(&["patch", "--dry-run"]);
    assert!(run.stdout.contains("(lightweight)"), "{}", run.stdout);

    let value = fx.run(&["patch", "--dry-run", "--json"]).json();
    assert_eq!(value["git"]["tag_style"], "lightweight");
}

#[test]
fn an_ordinary_annotated_tag_needs_no_remark() {
    let fx = taggable("through = \"tag\"\n");

    let run = fx.run(&["patch", "--dry-run"]);
    assert!(
        run.stdout.contains("would tag     v1.2.4"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("(annotated)"), "{}", run.stdout);
}

// ─── C# projects ─────────────────────────────────────────────────────────────

/// dotnet's own output, shared with the unit tests.
const CSPROJ: &str = include_str!("../src/domain/testdata/Demo.csproj");

#[test]
fn a_csproj_is_tracked_by_its_extension() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"Demo.csproj\"]\n")
        .write("Demo.csproj", CSPROJ);

    let run = fx.run(&["minor", "--through", "none"]);
    assert_eq!(run.code, 0, "{}", run.output());

    let out = fx.read("Demo.csproj");
    assert!(out.contains("<Version>1.3.0</Version>"), "{out}");
    // Everything else a project file calls a version stays put.
    assert!(
        out.contains("<AssemblyVersion>1.2.3.0</AssemblyVersion>"),
        "{out}"
    );
    assert!(out.contains(r#"Version="13.0.3""#), "{out}");
    assert!(out.starts_with('\u{feff}'), "byte-order mark lost");
}

#[test]
fn a_csproj_moves_in_step_with_other_tracked_files() {
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\", \"Demo.csproj\"]\n")
        .write("VERSION", "1.2.3\n")
        .write("Demo.csproj", CSPROJ);

    assert_eq!(
        fx.run(&["alpha", "--toward", "patch", "--through", "none"])
            .code,
        0
    );
    assert_eq!(fx.read("VERSION"), "1.2.4-alpha.0\n");
    assert!(
        fx.read("Demo.csproj")
            .contains("<Version>1.2.4-alpha.0</Version>")
    );
    assert_eq!(fx.run(&["check", "1.2.4-alpha.0"]).code, 0);
}

#[test]
fn init_finds_a_csproj_and_skips_one_with_no_version() {
    let fx = Fixture::new().write("src/Demo/Demo.csproj", CSPROJ).write(
        "src/Tools/Tools.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup></PropertyGroup></Project>\n",
    );

    assert_eq!(fx.run(&["init"]).code, 0);
    let config = fx.read("vump.toml");

    assert!(config.contains("src/Demo/Demo.csproj"), "{config}");
    // A project that declares no version cannot be kept in step with one.
    assert!(!config.contains("Tools.csproj"), "{config}");
}

#[test]
fn a_project_with_no_name_is_not_called_a_repository() {
    // A configuration describes projects, and a repository may hold several
    // configurations describing different things — so naming the one in effect
    // after the repository asserts a scope it does not have.
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n");

    let run = fx.run(&["status"]);
    assert!(run.stdout.contains("(unnamed project)"), "{}", run.stdout);
    assert!(!run.stdout.contains("repository"), "{}", run.stdout);
}

#[test]
fn projects_are_separated_only_where_files_are_listed() {
    // The blank line detaches a file listing from the row above it. Between
    // rows that are a single line each it separates nothing.
    let fx = Fixture::new()
        .write("vump.toml", MULTI)
        .write("api/Cargo.toml", "[package]\nversion = \"1.0.0\"\n")
        .write("web/package.json", "{\"version\":\"3.4.5\"}");

    let agreed = fx.run(&["status"]);
    assert_eq!(agreed.code, 0, "{}", agreed.output());
    assert!(!agreed.stdout.contains("\n\n"), "{:?}", agreed.stdout);

    // One project disagreeing brings the listing back, and the separator with
    // it: without the blank line the rows would run into the file list.
    let fx = fx.write("api/Cargo.toml", "[package]\nversion = \"1.0.0\"\n");
    let fx = fx.write(
        "vump.toml",
        "[[project]]\nname = \"api\"\nfiles = [\"api/Cargo.toml\", \"api/VERSION\"]\n\n         [[project]]\nname = \"web\"\nfiles = [\"web/package.json\"]\n",
    );
    let fx = fx.write("api/VERSION", "9.9.9\n");

    let split = fx.run(&["status"]);
    assert_eq!(split.code, 5, "{}", split.output());
    assert!(split.stdout.contains("\n\n"), "{:?}", split.stdout);
}

#[test]
fn a_refusal_names_both_configurations_in_full() {
    // Below two configurations, a path relative to the nearer one starts
    // partway down the tree and reads as if it started at the top. Naming both
    // files outright needs no frame, so there is no frame to read wrongly.
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n")
        .write("mid/vump.toml", SINGLE)
        .write("mid/VERSION", "4.5.6\n")
        .write("mid/deep/inner/vump.toml", SINGLE)
        .write("mid/deep/inner/VERSION", "9.9.9\n");

    let run = fx.run_in("mid/deep/inner", &["status"]);
    assert_eq!(run.code, 3, "{}", run.output());

    let (inner, rest) = run
        .stderr
        .strip_prefix("error: ")
        .and_then(|rest| rest.split_once(" sits below "))
        .expect("the refusal names the configuration in effect");
    let outer = rest
        .split_once(", so this acts")
        .expect("the refusal names the one it shadows")
        .0;

    // Absolute, so neither path is read against a directory the reader has to
    // guess at.
    assert!(Path::new(inner).is_absolute(), "{inner}");
    assert!(Path::new(outer).is_absolute(), "{outer}");

    let inner_tail = Path::new("mid")
        .join("deep")
        .join("inner")
        .join("vump.toml");
    let outer_tail = Path::new("mid").join("vump.toml");
    assert!(
        inner.ends_with(&inner_tail.display().to_string()),
        "{inner}"
    );
    assert!(
        outer.ends_with(&outer_tail.display().to_string()),
        "{outer}"
    );
}

#[test]
fn a_repository_of_its_own_does_not_excuse_a_nested_configuration() {
    // Nesting is a question about configurations, so the answer does not turn
    // on what the directory is to git. Two stacked configurations still cost
    // name addressing and a single tag namespace, which is what the refusal
    // points at — and the check reads the same files discovery does.
    let fx = Fixture::new()
        .write("vump.toml", SINGLE)
        .write("VERSION", "1.2.3\n")
        .write("inner/vump.toml", SINGLE)
        .write("inner/VERSION", "9.9.9\n")
        .with_git();

    assert_eq!(fx.run_in("inner", &["status"]).code, 3);

    fx.git_in("inner", &["init", "-q", "."]);
    assert_eq!(fx.run_in("inner", &["status"]).code, 3);
    assert_eq!(fx.run_in("inner", &["status", "--allow-nested"]).code, 0);
}

// ─── Release branches ────────────────────────────────────────────────────────

/// A repository that tags stable releases only from `main`, sitting on `feat/x`.
fn off_release_branch() -> Fixture {
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n\n[git]\nrelease_branches = [\"main\"]\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();
    fx.branch("feat/x");
    fx
}

#[test]
fn a_stable_release_from_an_unlisted_branch_is_refused() {
    // The accident this exists for: the pull request is merged and its branch
    // deleted upstream, but the local checkout is still on it, so the release
    // commit and tag land somewhere nothing will reach again.
    let fx = off_release_branch();

    let run = fx.run(&["patch", "--through", "tag"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("feat/x"), "{}", run.stderr);
    assert!(run.stderr.contains("main"), "{}", run.stderr);

    // Refusing must leave everything as it was, files included: the check runs
    // before the first write, not after it.
    assert_eq!(fx.read("VERSION"), "1.2.3\n");
    assert_eq!(fx.tags(), Vec::<String>::new());
}

#[test]
fn a_refused_release_is_refused_as_a_dry_run_too() {
    // --dry-run reports what a real run would do. A plan for a run that would
    // be refused is not that.
    let fx = off_release_branch();

    let run = fx.run(&["patch", "--through", "tag", "--dry-run"]);
    assert_eq!(run.code, 3, "{}", run.output());
}

#[test]
fn any_branch_proceeds_and_says_so() {
    // Waived, not silenced: the flag is an acknowledgement, so the run still
    // reports what was acknowledged.
    let fx = off_release_branch();

    let run = fx.run(&["patch", "--through", "tag", "--any-branch"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.tags(), ["v1.2.4"]);
    assert!(run.stderr.contains("feat/x"), "{}", run.stderr);
    assert!(run.stderr.contains("release_branches"), "{}", run.stderr);
}

#[test]
fn a_pre_release_is_not_governed_by_the_stable_list() {
    // Sharing unfinished work is what a pre-release is for. Constraining it
    // with the stable list would mean merging first, every time.
    let fx = off_release_branch();

    let run = fx.run(&["alpha", "--toward", "patch", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.tags(), ["v1.2.4-alpha.0"]);
}

#[test]
fn a_pre_release_can_be_constrained_on_its_own() {
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n\n[git]\nprerelease_branches = [\"develop\"]\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();
    fx.branch("feat/x");

    let run = fx.run(&["alpha", "--toward", "patch", "--through", "tag"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("develop"), "{}", run.stderr);

    // The stable list is unset, so stable releases stay unconstrained.
    assert_eq!(fx.run(&["patch", "--through", "tag"]).code, 0);
}

#[test]
fn a_listed_branch_releases() {
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n\n[git]\nrelease_branches = [\"main\", \"release\"]\n",
        )
        .write("VERSION", "1.2.3\n")
        .with_git();
    fx.branch("release");

    let run = fx.run(&["patch", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.tags(), ["v1.2.4"]);
}

#[test]
fn writing_files_without_git_is_not_governed() {
    // Nothing is left behind that could belong to the wrong branch, and
    // refusing it would block bumping a version inside a pull request — which
    // is where a version is supposed to change.
    let fx = off_release_branch();

    let run = fx.run(&["patch", "--through", "none"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.read("VERSION"), "1.2.4\n");
    assert_eq!(fx.tags(), Vec::<String>::new());
}

#[test]
fn a_detached_head_is_refused_where_a_policy_exists() {
    let fx = off_release_branch();
    fx.detach();

    let run = fx.run(&["patch", "--through", "tag"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("detached"), "{}", run.stderr);
    assert_eq!(fx.tags(), Vec::<String>::new());
}

#[test]
fn no_policy_leaves_every_branch_releasing() {
    // The check is opt-in in both directions: without it, a detached HEAD is
    // nobody's problem but the caller's.
    let fx = Fixture::new()
        .write("vump.toml", "files = [\"VERSION\"]\n")
        .write("VERSION", "1.2.3\n")
        .with_git();
    fx.branch("feat/x");

    let run = fx.run(&["patch", "--through", "tag"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fx.tags(), ["v1.2.4"]);
}

#[test]
fn an_empty_branch_list_is_refused_before_anything_runs() {
    let fx = Fixture::new()
        .write(
            "vump.toml",
            "files = [\"VERSION\"]\n\n[git]\nrelease_branches = []\n",
        )
        .write("VERSION", "1.2.3\n");

    let run = fx.run(&["status"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.stderr.contains("release_branches"), "{}", run.stderr);
}

// ─── Sandbox ─────────────────────────────────────────────────────────────────

#[test]
fn the_sandbox_projects_stay_usable() {
    // The sandbox is committed as worked examples, so it must not be allowed to
    // drift into demonstrating something that no longer holds. `status`
    // succeeds only when every tracked file of every project agrees, which is
    // also true after a bump — so experimenting there does not fail this.
    let sandbox = Path::new(env!("CARGO_MANIFEST_DIR")).join("sandbox");

    for project in [
        "npm/single-project",
        "npm/multi-project",
        "cs/single-project",
        "cs/multi-project",
        "cs/shared-version",
        "py/single-project",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_vump"))
            .args(["status", "--allow-nested"])
            .current_dir(sandbox.join(project))
            .stdin(Stdio::null())
            .output()
            .expect("cannot run vump");

        assert!(
            output.status.success(),
            "{project}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn no_sandbox_project_can_produce_a_release_shaped_tag() {
    // These projects sit inside vump's own repository, so a tag made in one
    // lands here. The release workflow triggers on `v*`, and `vump patch
    // --through push` run in a sandbox directory by mistake once created a
    // `v1.0.1` tag that looked exactly like a release of vump itself.
    //
    // Asserting on the rendered tag rather than on the configured pattern is
    // deliberate: deleting the pattern silently restores the "v{new_version}"
    // default, which is the very shape being excluded.
    let sandbox = Path::new(env!("CARGO_MANIFEST_DIR")).join("sandbox");

    for (project, select) in [
        ("npm/single-project", None),
        ("npm/multi-project", Some("project-a")),
        ("cs/single-project", None),
        ("cs/multi-project", Some("project-b")),
        ("cs/shared-version", None),
        ("py/single-project", None),
    ] {
        // --allow-nested is the point of this test: it asks for the tag a
        // nested configuration would create, precisely to inspect its shape.
        let mut args = vec![
            "patch",
            "--through",
            "tag",
            "--dry-run",
            "--json",
            "--allow-nested",
        ];
        if let Some(name) = select {
            args.extend(["--project", name]);
        }

        let output = Command::new(env!("CARGO_BIN_EXE_vump"))
            .args(&args)
            .current_dir(sandbox.join(project))
            .stdin(Stdio::null())
            .output()
            .expect("cannot run vump");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let value: serde_json::Value =
            serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{project}: {e}: {stdout}"));

        let tags = collect_tags(&value);
        assert!(!tags.is_empty(), "{project}: no tag in {stdout}");
        for tag in tags {
            assert!(
                tag.starts_with("sandbox-"),
                "{project}: tag {tag:?} must be prefixed so it cannot be taken \
                 for a release of vump",
            );
        }
    }
}

/// Every tag name anywhere in a JSON result.
///
/// The shape differs between a single project and several, and this assertion
/// is about all of them regardless of where they sit.
fn collect_tags(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .flat_map(|(key, child)| match (key.as_str(), child.as_str()) {
                ("tag", Some(name)) => vec![name.to_owned()],
                _ => collect_tags(child),
            })
            .collect(),
        serde_json::Value::Array(items) => items.iter().flat_map(collect_tags).collect(),
        _ => Vec::new(),
    }
}

// ─── Every supported format, through the real binary ────────────────────────

/// One supported layout: the files a project of that shape has, and what
/// `vump.toml` declares.
struct FormatCase {
    /// Named in failure messages.
    label: &'static str,
    /// `files = [...]`, in configuration order.
    declared: &'static [&'static str],
    /// Files to write, each recording `1.0.0`.
    contents: &'static [(&'static str, &'static str)],
}

/// Every format vump recognizes, as a project that has it.
///
/// Adding a format means adding a row. The alternative is what this replaced:
/// building one project per ecosystem by hand and reading the result, which is
/// neither repeatable nor run on three platforms.
const FORMAT_CASES: &[FormatCase] = &[
    FormatCase {
        label: "plain text",
        declared: &["VERSION"],
        contents: &[("VERSION", "1.0.0\n")],
    },
    FormatCase {
        label: "npm manifest",
        declared: &["package.json"],
        contents: &[(
            "package.json",
            "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n",
        )],
    },
    FormatCase {
        label: "npm manifest and lock",
        declared: &["package.json", "package-lock.json"],
        contents: &[
            (
                "package.json",
                "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n",
            ),
            (
                "package-lock.json",
                "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\",\n  \"lockfileVersion\": 3,\n  \"packages\": {\n    \"\": {\n      \"name\": \"demo\",\n      \"version\": \"1.0.0\"\n    }\n  }\n}\n",
            ),
        ],
    },
    FormatCase {
        label: "cargo manifest",
        declared: &["Cargo.toml"],
        contents: &[(
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
        )],
    },
    FormatCase {
        label: "cargo manifest and lock",
        declared: &["Cargo.toml", "Cargo.lock"],
        contents: &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
            ),
            (
                "Cargo.lock",
                "version = 3\n\n[[package]]\nname = \"demo\"\nversion = \"1.0.0\"\n",
            ),
        ],
    },
    FormatCase {
        label: "python manifest",
        declared: &["pyproject.toml"],
        contents: &[(
            "pyproject.toml",
            "[project]\nname = \"demo\"\nversion = \"1.0.0\"\n",
        )],
    },
    FormatCase {
        label: "python manifest and uv lock",
        declared: &["pyproject.toml", "uv.lock"],
        contents: &[
            (
                "pyproject.toml",
                "[project]\nname = \"demo\"\nversion = \"1.0.0\"\n",
            ),
            (
                "uv.lock",
                "version = 1\nrevision = 3\n\n[[package]]\nname = \"demo\"\nversion = \"1.0.0\"\nsource = { editable = \".\" }\n",
            ),
        ],
    },
    FormatCase {
        label: "msbuild project",
        declared: &["Demo.csproj"],
        contents: &[(
            "Demo.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <Version>1.0.0</Version>\n  </PropertyGroup>\n</Project>\n",
        )],
    },
    FormatCase {
        label: "msbuild shared props",
        declared: &["Directory.Build.props"],
        contents: &[(
            "Directory.Build.props",
            "<Project>\n  <PropertyGroup>\n    <Version>1.0.0</Version>\n  </PropertyGroup>\n</Project>\n",
        )],
    },
    FormatCase {
        label: "msbuild shared targets",
        declared: &["Directory.Build.targets"],
        contents: &[(
            "Directory.Build.targets",
            "<Project>\n  <PropertyGroup>\n    <Version>1.0.0</Version>\n  </PropertyGroup>\n</Project>\n",
        )],
    },
];

#[test]
fn every_supported_format_round_trips_through_the_binary() {
    for case in FORMAT_CASES {
        let label = case.label;
        let declared = case
            .declared
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");

        let mut fx = Fixture::new().write("vump.toml", &format!("files = [{declared}]\n"));
        for (name, body) in case.contents {
            fx = fx.write(name, body);
        }

        let status = fx.run(&["status"]);
        assert!(status.code == 0, "{label}: status: {}", status.output());
        assert!(
            status.stdout.contains("1.0.0"),
            "{label}: {}",
            status.stdout
        );

        let check = fx.run(&["check", "1.0.0"]);
        assert_eq!(check.code, 0, "{label}: check: {}", check.output());

        let bump = fx.run(&["patch", "--through", "none"]);
        assert_eq!(bump.code, 0, "{label}: patch: {}", bump.output());

        // Every declared file moved, not just the first one read.
        for (name, _) in case.contents {
            assert!(
                fx.read(name).contains("1.0.1"),
                "{label}: {name} still holds the old version:\n{}",
                fx.read(name),
            );
        }

        // `check` is the contract CI depends on, so it is asserted in both
        // directions: agreeing with the new version and refusing the old. The
        // code is 4 and not 5 — the files agree with each other and disagree
        // with the argument, which is a mismatch rather than being out of sync.
        assert_eq!(
            fx.run(&["check", "1.0.1"]).code,
            0,
            "{label}: check after bump"
        );
        assert_eq!(
            fx.run(&["check", "1.0.0"]).code,
            4,
            "{label}: a stale version is a mismatch",
        );
    }
}

#[test]
fn every_supported_format_reaches_a_pre_release_and_back() {
    // The pre-release path writes a version that is not three numbers, which
    // is where a format that cannot hold one shows up — an <AssemblyVersion>
    // takes four numeric parts and would silently truncate.
    for case in FORMAT_CASES {
        let label = case.label;
        let declared = case
            .declared
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");

        let mut fx = Fixture::new().write("vump.toml", &format!("files = [{declared}]\n"));
        for (name, body) in case.contents {
            fx = fx.write(name, body);
        }

        let alpha = fx.run(&["alpha", "--toward", "minor", "--through", "none"]);
        assert_eq!(alpha.code, 0, "{label}: alpha: {}", alpha.output());
        for (name, _) in case.contents {
            assert!(
                fx.read(name).contains("1.1.0-alpha.0"),
                "{label}: {name}:\n{}",
                fx.read(name),
            );
        }

        let release = fx.run(&["release", "--through", "none"]);
        assert_eq!(release.code, 0, "{label}: release: {}", release.output());
        for (name, _) in case.contents {
            assert!(fx.read(name).contains("1.1.0"), "{label}: {name}");
        }
    }
}
