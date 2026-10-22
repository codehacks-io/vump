//! Creating an initial configuration.
//!
//! Discovery is deliberately dumb: it finds every recognized version file and
//! writes them all out. Deciding which of them belong together, or splitting
//! them into named projects, is left to the person editing the handful of lines
//! this produces — a task the tool cannot do better than they can, and one that
//! would otherwise cost a wizard's worth of questions on a file written once.

use std::path::Path;

use thiserror::Error;

use crate::config::{DEFAULT_COMMIT_MESSAGE, DEFAULT_TAG_MESSAGE, DEFAULT_TAG_PATTERN, FILE_NAME};
use crate::domain::version_file::Tracked;
use crate::ports::{FileSystem, FsError};

/// How deep beneath the starting directory to look.
///
/// Deep enough for the usual `apps/<name>/package.json` layout, shallow enough
/// not to crawl an entire checkout.
const MAX_DEPTH: usize = 3;

/// Directories never descended into.
///
/// These hold dependencies and build output, whose manifests describe other
/// people's software rather than this repository's.
const SKIPPED: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    "coverage",
    ".git",
];

/// Why an initial configuration could not be created.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InitError {
    /// A configuration already exists.
    #[error("{path} already exists; pass --force to overwrite it")]
    AlreadyExists {
        /// Path of the existing configuration.
        path: String,
    },

    /// Nothing recognizable was found to track.
    #[error(
        "no version files found under {root}; vump recognizes package.json, Cargo.toml and VERSION"
    )]
    NothingFound {
        /// Directory that was searched.
        root: String,
    },

    /// The configuration could not be written.
    #[error("{0}")]
    Filesystem(#[from] FsError),
}

/// Finds every recognized version file beneath `root`.
///
/// Paths are returned relative to `root`, using forward slashes, ordered
/// shallowest first so the repository's own manifest leads.
pub fn discover(fs: &dyn FileSystem, root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    walk(fs, root, "", 0, &mut found);
    // Shallower paths first, then alphabetical, giving a stable, readable list.
    found.sort_by(|a: &String, b: &String| {
        let depth = |p: &str| p.matches('/').count();
        depth(a).cmp(&depth(b)).then_with(|| a.cmp(b))
    });

    // A lock file's entries are identified by the manifests declared with it,
    // so every manifest has to be in hand before anything can be judged.
    let packages = crate::app::local_package_names(fs, root, found.iter().map(String::as_str));

    found.retain(|path| readable(fs, root, path, &packages));
    found
}

fn walk(fs: &dyn FileSystem, dir: &Path, prefix: &str, depth: usize, found: &mut Vec<String>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs.read_dir(dir) else {
        return;
    };

    for entry in entries {
        if entry.is_dir {
            // Hidden directories hold tooling state, not project manifests.
            if SKIPPED.contains(&entry.name.as_str()) || entry.name.starts_with('.') {
                continue;
            }
            let child_prefix = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            walk(fs, &dir.join(&entry.name), &child_prefix, depth + 1, found);
        } else if Tracked::detect(&entry.name).is_some() {
            found.push(if prefix.is_empty() {
                entry.name
            } else {
                format!("{prefix}/{}", entry.name)
            });
        }
    }
}

/// Whether a discovered file carries a version vump could keep in step.
///
/// A virtual workspace manifest declares no package, a private `package.json`
/// often carries no version, and a lock file naming none of the packages found
/// beside it belongs to something else. Declaring any of them would produce a
/// configuration that fails on every command it is used for.
fn readable(fs: &dyn FileSystem, root: &Path, declared: &str, packages: &[String]) -> bool {
    let absolute = crate::app::resolve(root, declared);
    let Some(tracked) = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(Tracked::detect)
    else {
        return false;
    };

    fs.read(&absolute).is_ok_and(|contents| match tracked {
        Tracked::Manifest(format) => format.read(declared, &contents).is_ok(),
        Tracked::Lock(lock) => lock.read(declared, &contents, packages).is_ok(),
    })
}

/// Writes an initial configuration tracking every version file found.
///
/// # Errors
///
/// Returns an [`InitError`] when a configuration already exists and `force` is
/// not set, when nothing was found, or when the file cannot be written.
pub fn init(fs: &dyn FileSystem, root: &Path, force: bool) -> Result<Vec<String>, InitError> {
    let target = root.join(FILE_NAME);

    if fs.is_file(&target) && !force {
        return Err(InitError::AlreadyExists {
            path: FILE_NAME.to_owned(),
        });
    }

    let files = discover(fs, root);
    if files.is_empty() {
        return Err(InitError::NothingFound {
            root: root.display().to_string(),
        });
    }

    fs.write(&target, &render(&files))?;
    Ok(files)
}

/// Produces the configuration text.
///
/// Git settings are written out commented, at their defaults, so that turning
/// one on is a matter of deleting a `#` rather than remembering the key's name.
fn render(files: &[String]) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "# Generated by `vump init`. Edit freely.");
    let _ = writeln!(out, "#");
    let _ = writeln!(
        out,
        "# Every file listed here is kept at the same version. To version parts"
    );
    let _ = writeln!(
        out,
        "# of this repository independently, replace `files` with named projects:"
    );
    let _ = writeln!(out, "#");
    let _ = writeln!(out, "#   [[project]]");
    let _ = writeln!(out, "#   name = \"api\"");
    let _ = writeln!(out, "#   files = [\"services/api/Cargo.toml\"]");
    let _ = writeln!(out);

    let _ = writeln!(out, "files = [");
    for file in files {
        let _ = writeln!(out, "    \"{file}\",");
    }
    let _ = writeln!(out, "]");
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "# How far a bump carries the release. Each step performs"
    );
    let _ = writeln!(
        out,
        "# every step before it, so \"tag\" also commits. This is a"
    );
    let _ = writeln!(
        out,
        "# decision, not a default: it is carried out without asking."
    );
    let _ = writeln!(out, "[git]");
    let _ = writeln!(
        out,
        "through = \"tag\"   # or \"none\", \"commit\", \"push\""
    );
    let _ = writeln!(out, "# commit_message = \"{DEFAULT_COMMIT_MESSAGE}\"");
    let _ = writeln!(out, "# tag_pattern = \"{DEFAULT_TAG_PATTERN}\"");
    let _ = writeln!(out, "# tag_message = \"{DEFAULT_TAG_MESSAGE}\"");
    let _ = writeln!(
        out,
        "# tag_style = \"annotated\"   # or \"lightweight\", or \"signed\""
    );
    let _ = writeln!(out);
    // Commented rather than filled in: which branch releases is policy, and
    // whichever branch `init` happened to run from is an accident, not a
    // declaration of one.
    let _ = writeln!(
        out,
        "# Branches a release may be tagged from. Unset means any branch."
    );
    let _ = writeln!(out, "# release_branches = [\"main\"]");
    let _ = writeln!(out, "# prerelease_branches = [\"main\"]");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::MemoryFileSystem;
    use crate::config::{Config, GitThrough};

    fn root() -> &'static Path {
        Path::new("/repo")
    }

    #[test]
    fn finds_recognized_manifests_at_any_depth() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.0.0\n")
            .with_file("/repo/apps/web/package.json", r#"{"version":"1.0.0"}"#)
            .with_file(
                "/repo/services/api/Cargo.toml",
                "[package]\nversion=\"1.0.0\"\n",
            );

        assert_eq!(
            discover(&fs, root()),
            [
                "VERSION",
                "apps/web/package.json",
                "services/api/Cargo.toml"
            ]
        );
    }

    #[test]
    fn ignores_dependency_and_build_directories() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/package.json", r#"{"version":"1.0.0"}"#)
            .with_file("/repo/node_modules/left-pad/package.json", "{}")
            .with_file("/repo/target/debug/Cargo.toml", "[package]")
            .with_file("/repo/.git/config", "");

        // A dependency's manifest describes someone else's software.
        assert_eq!(discover(&fs, root()), ["package.json"]);
    }

    #[test]
    fn ignores_unrecognized_files() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/pyproject.toml", "")
            .with_file("/repo/README.md", "");

        assert_eq!(discover(&fs, root()), Vec::<String>::new());
    }

    #[test]
    fn writes_a_configuration_that_parses_back() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.0.0\n")
            .with_file("/repo/ui/package.json", r#"{"version":"1.0.0"}"#);

        let written = init(&fs, root(), false).unwrap();
        assert_eq!(written, ["VERSION", "ui/package.json"]);

        // The generated file must be valid input to the very parser that will
        // read it back on the next run.
        let text = fs.get("/repo/vump.toml").expect("config must be written");
        let config = Config::parse(Path::new("vump.toml"), &text).expect("generated config parses");

        assert!(config.is_single_unnamed());
        assert_eq!(config.projects[0].files, ["VERSION", "ui/package.json"]);
        // The generated default stops short of the one step that reaches
        // other people.
        assert_eq!(config.git.through, Some(GitThrough::Tag));
        assert!(!config.git.through.unwrap().pushes());
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.0.0\n")
            .with_file("/repo/vump.toml", "files = [\"VERSION\"]\n");

        let err = init(&fs, root(), false).unwrap_err();
        assert!(matches!(err, InitError::AlreadyExists { .. }));

        assert!(init(&fs, root(), true).is_ok());
    }

    #[test]
    fn reports_when_there_is_nothing_to_track() {
        let fs = MemoryFileSystem::new().with_file("/repo/README.md", "");
        let err = init(&fs, root(), false).unwrap_err();
        assert!(matches!(err, InitError::NothingFound { .. }));
    }
}
