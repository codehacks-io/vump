//! Version control adapter backed by the `git` executable.
//!
//! Shelling out rather than linking a git library is deliberate: it inherits
//! the user's real configuration, credential helpers, hooks and SSH setup for
//! free, and keeps the dependency surface small.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ports::{Annotation, Vcs, VcsError, WorkingTree};

/// Runs `git` against one repository.
#[derive(Debug, Clone)]
pub struct GitCli {
    root: PathBuf,
}

impl GitCli {
    /// Operates on the repository rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Runs a git subcommand, returning its standard output on success.
    fn run(&self, operation: &str, args: &[&str]) -> Result<String, VcsError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| VcsError::Unavailable {
                detail: e.to_string(),
            })?;

        if !output.status.success() {
            // git reports actionable detail on stderr, but falls back to stdout
            // for some subcommands, so both are considered.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            return Err(VcsError::Failed {
                operation: operation.to_owned(),
                detail: detail.to_owned(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Vcs for GitCli {
    fn status(&self) -> Result<WorkingTree, VcsError> {
        let out = self.run("status", &["status", "--porcelain"])?;
        Ok(WorkingTree {
            changed: parse_porcelain(&out),
        })
    }

    fn current_branch(&self) -> Result<Option<String>, VcsError> {
        // --show-current prints an empty line and succeeds on a detached HEAD,
        // where symbolic-ref would exit non-zero and be reported as a failure.
        let out = self.run("branch", &["branch", "--show-current"])?;
        let name = out.trim();
        Ok((!name.is_empty()).then(|| name.to_owned()))
    }

    fn stage(&self, paths: &[String]) -> Result<(), VcsError> {
        let mut args = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run("add", &args)?;
        Ok(())
    }

    fn commit(&self, message: &str) -> Result<(), VcsError> {
        self.run("commit", &["commit", "-m", message])?;
        Ok(())
    }

    fn tag(&self, name: &str, annotation: Option<&Annotation>) -> Result<(), VcsError> {
        let mut args = vec!["tag"];
        if let Some(annotation) = annotation {
            // -s implies an annotated tag, so the two flags are exclusive.
            args.push(if annotation.signed { "-s" } else { "-a" });
            args.push("-m");
            args.push(&annotation.message);
        }
        args.push(name);

        self.run("tag", &args)?;
        Ok(())
    }

    fn push(&self, tag: Option<&str>) -> Result<(), VcsError> {
        self.run("push", &["push"])?;
        if let Some(tag) = tag {
            self.run("push", &["push", "origin", tag])?;
        }
        Ok(())
    }
}

/// Extracts paths from `git status --porcelain` output.
///
/// Each line is two status characters, a space, then the path. Renames are
/// reported as `old -> new`; the new path is the one that matters here.
fn parse_porcelain(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let path = line.get(3..)?.trim();
            if path.is_empty() {
                return None;
            }
            Some(
                path.split_once(" -> ")
                    .map_or(path, |(_, new)| new)
                    .trim_matches('"')
                    .to_owned(),
            )
        })
        .collect()
}

/// Locates the repository root containing `start`.
///
/// # Errors
///
/// Returns [`VcsError`] when `start` is not inside a git repository.
pub fn discover_root(start: &Path) -> Result<PathBuf, VcsError> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .map_err(|e| VcsError::Unavailable {
            detail: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(VcsError::Failed {
            operation: "rev-parse".to_owned(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modified_and_untracked_entries() {
        let out = " M src/main.rs\n?? notes.txt\nA  Cargo.toml\n";
        assert_eq!(
            parse_porcelain(out),
            ["src/main.rs", "notes.txt", "Cargo.toml"]
        );
    }

    #[test]
    fn reports_the_destination_of_a_rename() {
        let out = "R  old/path.rs -> new/path.rs\n";
        assert_eq!(parse_porcelain(out), ["new/path.rs"]);
    }

    #[test]
    fn a_clean_tree_yields_nothing() {
        assert_eq!(parse_porcelain(""), Vec::<String>::new());
        assert_eq!(parse_porcelain("\n"), Vec::<String>::new());
    }

    #[test]
    fn quoted_paths_are_unwrapped() {
        // git quotes paths containing unusual characters.
        let out = " M \"src/a b.rs\"\n";
        assert_eq!(parse_porcelain(out), ["src/a b.rs"]);
    }
}
