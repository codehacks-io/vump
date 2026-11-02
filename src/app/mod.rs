//! Use cases.
//!
//! Each use case orchestrates the domain and the ports to carry out one
//! complete operation. Use cases depend on port traits only, so they can be
//! driven entirely from memory in tests.

pub mod bump;
pub mod change;
pub mod check;
pub mod guided;
pub mod init;
pub mod push;
pub mod set;
pub mod status;
pub mod update;

use std::path::{Path, PathBuf};

use semver::Version;
use thiserror::Error;

use crate::config::{Config, ConfigError, Project};
use crate::domain::version_file::{self, LockFile, Tracked, VersionFileError};
use crate::ports::{FileSystem, FsError};

/// The nearest `vump.toml` at or above `from`.
///
/// The only upward walk in the program. Both questions asked about
/// configuration are this one walk started from different places — which
/// configuration governs a directory, and what that configuration shadowed —
/// so they share the implementation rather than each keeping a copy. Two
/// copies is what let them drift into disagreeing about where to stop.
///
/// Nothing bounds the walk. A repository root would bound it, but that is a
/// question about version control, and which file governs a directory is not.
#[must_use]
pub fn locate(fs: &dyn FileSystem, from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .map(|dir| dir.join(crate::config::FILE_NAME))
        .find(|candidate| fs.is_file(candidate))
}

/// Loads the configuration governing `start`.
///
/// Returns the directory containing the file alongside the parsed
/// configuration. Paths declared in the configuration are relative to that
/// directory, not to the working directory.
///
/// # Errors
///
/// Returns a [`ConfigError`] when no configuration is found at or above
/// `start`, or when the one found is unreadable or invalid.
pub fn discover(fs: &dyn FileSystem, start: &Path) -> Result<(PathBuf, Config), ConfigError> {
    let Some(file) = locate(fs, start) else {
        return Err(ConfigError::NotFound {
            start: start.to_path_buf(),
        });
    };

    let text = fs.read(&file).map_err(|e| ConfigError::Unreadable {
        path: file.clone(),
        detail: e.to_string(),
    })?;
    let config = Config::parse(&file, &text)?;

    // The directory, not the file: declared paths resolve against it.
    let root = file
        .parent()
        .map_or_else(|| start.to_path_buf(), Path::to_path_buf);
    Ok((root, config))
}

/// The nearest `vump.toml` above the one in `root`, if there is one.
///
/// Discovery takes the first configuration it finds, so one deeper in the tree
/// shadows every one above it. Asking what it shadowed is the same search,
/// started one directory higher.
#[must_use]
pub fn outer_config(fs: &dyn FileSystem, root: &Path) -> Option<PathBuf> {
    locate(fs, root.parent()?)
}

/// A version file and the version currently recorded in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    /// Path as declared in configuration, relative to the repository root.
    pub path: String,
    /// The version read from the file.
    pub version: Version,
}

/// Why a use case could not complete.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppError {
    /// A declared file does not exist.
    #[error("{path} is declared in {config} but does not exist", config = crate::config::FILE_NAME)]
    MissingFile {
        /// The declared path.
        path: String,
    },

    /// A declared file could not be read.
    #[error("{0}")]
    Filesystem(#[from] FsError),

    /// A declared file could not be interpreted.
    #[error("{0}")]
    VersionFile(#[from] VersionFileError),

    /// A lock file records this project's version but is not tracked.
    #[error(
        "{lock} records this project's version but is not declared in {config}; \
         add it to files, or writing {manifest} will leave the two disagreeing",
        config = crate::config::FILE_NAME
    )]
    UndeclaredLock {
        /// The lock file found next to a declared manifest.
        lock: String,
        /// The manifest it belongs to.
        manifest: String,
    },
}

/// Reads the version recorded in every file of `project`.
///
/// Paths are resolved relative to `root`, the directory containing the
/// configuration file.
///
/// # Errors
///
/// Returns an [`AppError`] when any declared file is missing, unreadable, or
/// does not carry a usable version.
pub fn read_project_versions(
    fs: &dyn FileSystem,
    root: &Path,
    project: &Project,
) -> Result<Vec<FileVersion>, AppError> {
    let packages = local_package_names(fs, root, project.files.iter().map(String::as_str));
    let mut versions = Vec::with_capacity(project.files.len());

    for declared in &project.files {
        let absolute = resolve(root, declared);

        let file_name = absolute
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(declared.as_str());
        let tracked = Tracked::require(file_name)?;

        if !fs.is_file(&absolute) {
            return Err(AppError::MissingFile {
                path: declared.clone(),
            });
        }

        let contents = fs.read(&absolute)?;
        let version = match tracked {
            Tracked::Manifest(format) => format.read(declared, &contents)?,
            Tracked::Lock(lock) => lock.read(declared, &contents, &packages)?,
        };

        versions.push(FileVersion {
            path: declared.clone(),
            version,
        });
    }

    check_declared_locks(fs, root, project, &packages)?;

    Ok(versions)
}

/// The one version every file records, or the disagreement that prevents it.
///
/// Returns the evidence rather than an error type: the callers report a
/// disagreement differently — a bump refuses, a push refuses, `status` renders
/// it — and the shape of the finding is the only part they share.
///
/// # Errors
///
/// Returns every file and the version it records when they do not all agree.
pub fn agreed_version(files: &[FileVersion]) -> Result<Version, Vec<(String, Version)>> {
    let mut distinct: Vec<&Version> = Vec::new();
    for file in files {
        if !distinct.contains(&&file.version) {
            distinct.push(&file.version);
        }
    }

    match distinct.as_slice() {
        [only] => Ok((*only).clone()),
        _ => Err(files
            .iter()
            .map(|f| (f.path.clone(), f.version.clone()))
            .collect()),
    }
}

/// The package names this project's manifests declare.
///
/// A shared workspace lock records every member it builds, so the entries a
/// project means are the ones its own manifests name. Cargo and uv both work
/// this way and are read together, since one project can hold both. Manifests
/// that cannot be read are skipped here and reported by the pass that reads
/// their version.
pub(crate) fn local_package_names<'a>(
    fs: &dyn FileSystem,
    root: &Path,
    declared: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    declared
        .into_iter()
        .filter_map(|path| {
            let read = match file_name_of(path) {
                "package.json" => version_file::package_json_name,
                "Cargo.toml" => version_file::cargo_package_name,
                "pyproject.toml" => version_file::pyproject_package_name,
                _ => return None,
            };
            read(&fs.read(&resolve(root, path)).ok()?)
        })
        .collect()
}

/// The final component of a configured path.
fn file_name_of(declared: &str) -> &str {
    declared.rsplit_once('/').map_or(declared, |(_, name)| name)
}

/// Refuses a lock file that records this project's version but is not declared.
///
/// Writing a manifest without its lock leaves the two disagreeing, and
/// `cargo build --locked` rejects that outright — so a tag would be published
/// for a tree that cannot be built from it. Catching it here means the refusal
/// arrives before anything is written, rather than as advice after a commit and
/// tag already exist.
///
/// A lock vump cannot write is passed over rather than demanded: a workspace
/// lock covers several packages, and asking for it to be declared would only
/// produce a second refusal.
fn check_declared_locks(
    fs: &dyn FileSystem,
    root: &Path,
    project: &Project,
    packages: &[String],
) -> Result<(), AppError> {
    for declared in &project.files {
        let Some((candidates, lock)) = companion_locks(declared) else {
            continue;
        };

        for path in candidates {
            if project.files.contains(&path) {
                break;
            }

            let absolute = resolve(root, &path);
            if !fs.is_file(&absolute) {
                continue;
            }

            // Reading it decides whether it is one vump could keep in step.
            let contents = fs.read(&absolute)?;
            if lock.read(&path, &contents, packages).is_ok() {
                return Err(AppError::UndeclaredLock {
                    lock: path,
                    manifest: declared.clone(),
                });
            }
            break;
        }
    }

    Ok(())
}

/// Names the lock file that sits beside `declared` and records its version.
///
/// Only the lock files that record the project's own version are listed. A
/// `yarn.lock` or `pnpm-lock.yaml` carries none — Yarn pins its workspace at a
/// placeholder precisely so a bump does not churn the file — so a bump never
/// invalidates one.
fn companion_locks(declared: &str) -> Option<(Vec<String>, LockFile)> {
    let (directory, name) = declared
        .rsplit_once('/')
        .map_or(("", declared), |(dir, name)| (dir, name));

    let (lock, format) = match name {
        "Cargo.toml" => ("Cargo.lock", LockFile::Cargo),
        "package.json" => ("package-lock.json", LockFile::Npm),
        "pyproject.toml" => ("uv.lock", LockFile::Uv),
        _ => return None,
    };

    // Nearest first: a workspace shares one lock at its root, so a member's
    // lock may sit any number of directories above the manifest.
    let mut candidates = Vec::new();
    let mut current = Some(directory);
    while let Some(dir) = current {
        candidates.push(if dir.is_empty() {
            lock.to_owned()
        } else {
            format!("{dir}/{lock}")
        });
        current = if dir.is_empty() {
            None
        } else {
            Some(dir.rsplit_once('/').map_or("", |(parent, _)| parent))
        };
    }

    Some((candidates, format))
}

/// Resolves a configured path against the configuration's directory.
///
/// Configured paths always use forward slashes, so they are split explicitly
/// rather than handed to the platform's path parser.
#[must_use]
pub fn resolve(root: &Path, declared: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for segment in declared.split('/').filter(|s| !s.is_empty() && *s != ".") {
        path.push(segment);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::MemoryFileSystem;

    fn project(files: &[&str]) -> Project {
        Project {
            name: None,
            files: files.iter().map(|s| (*s).to_owned()).collect(),
            tag_pattern: None,
        }
    }

    #[test]
    fn discovery_takes_the_nearest_configuration() {
        let fs = MemoryFileSystem::new()
            .with_file("/repo/vump.toml", "files = [\"VERSION\"]\n")
            .with_file("/repo/inner/vump.toml", "files = [\"VERSION\"]\n");

        let (root, _) = discover(&fs, Path::new("/repo/inner/src")).unwrap();
        assert_eq!(root, Path::new("/repo/inner"));
    }

    #[test]
    fn discovery_walks_up_until_it_finds_one() {
        let fs = MemoryFileSystem::new().with_file("/repo/vump.toml", "files = [\"VERSION\"]\n");

        let (root, _) = discover(&fs, Path::new("/repo/a/b/c")).unwrap();
        assert_eq!(root, Path::new("/repo"));
    }

    #[test]
    fn no_configuration_anywhere_is_reported_as_missing() {
        let fs = MemoryFileSystem::new();
        let err = discover(&fs, Path::new("/repo/a")).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound { .. }), "got {err:?}");
    }

    #[test]
    fn an_outer_configuration_is_the_same_search_started_higher() {
        // The two questions asked about configuration are one walk from two
        // starting points, so they cannot disagree about where a search stops.
        let fs = MemoryFileSystem::new()
            .with_file("/repo/vump.toml", "files = [\"VERSION\"]\n")
            .with_file("/repo/inner/vump.toml", "files = [\"VERSION\"]\n");

        let inner = Path::new("/repo/inner");
        assert_eq!(locate(&fs, inner).unwrap(), inner.join("vump.toml"));
        assert_eq!(
            outer_config(&fs, inner).unwrap(),
            Path::new("/repo/vump.toml")
        );
        assert_eq!(outer_config(&fs, Path::new("/repo")), None);
    }

    #[test]
    fn nesting_does_not_depend_on_version_control() {
        // Nothing bounds the walk, so a directory that git would call a
        // repository root reads exactly like any other directory.
        let fs = MemoryFileSystem::new()
            .with_file("/repo/vump.toml", "files = [\"VERSION\"]\n")
            .with_file("/repo/inner/.git/HEAD", "ref: refs/heads/main\n")
            .with_file("/repo/inner/vump.toml", "files = [\"VERSION\"]\n");

        assert_eq!(
            outer_config(&fs, Path::new("/repo/inner")).unwrap(),
            Path::new("/repo/vump.toml")
        );
    }

    #[test]
    fn reads_every_declared_file() {
        let root = Path::new("/repo");
        let fs = MemoryFileSystem::new()
            .with_file("/repo/VERSION", "1.2.3\n")
            .with_file("/repo/ui/package.json", r#"{"version":"1.2.3"}"#);

        let versions =
            read_project_versions(&fs, root, &project(&["VERSION", "ui/package.json"])).unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].path, "VERSION");
        assert_eq!(versions[1].version, "1.2.3".parse().unwrap());
    }

    #[test]
    fn a_missing_file_is_reported_with_its_declared_path() {
        let fs = MemoryFileSystem::new();
        let err =
            read_project_versions(&fs, Path::new("/repo"), &project(&["VERSION"])).unwrap_err();

        // The declared path is what the user wrote in configuration, so that is
        // what the message must name.
        assert!(matches!(err, AppError::MissingFile { path } if path == "VERSION"));
    }

    #[test]
    fn an_unsupported_filename_is_rejected_before_any_read() {
        let fs = MemoryFileSystem::new();
        let err =
            read_project_versions(&fs, Path::new("/repo"), &project(&["setup.py"])).unwrap_err();
        assert!(matches!(
            err,
            AppError::VersionFile(VersionFileError::UnsupportedFile { .. })
        ));
    }

    /// A lock naming each of `members` as built from this repository.
    fn lock(members: &[&str]) -> String {
        let mut out = String::from(
            "version = 4\n\n\
             [[package]]\nname = \"semver\"\nversion = \"1.0.23\"\n\
             source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        );
        for name in members {
            out.push_str("\n[[package]]\nname = \"");
            out.push_str(name);
            out.push_str("\"\nversion = \"1.2.3\"\n");
        }
        out
    }

    #[test]
    fn a_lock_recording_this_version_must_be_declared() {
        // Writing the manifest without it leaves `cargo build --locked` failing
        // on a tree a tag already claims is correct.
        let fs = MemoryFileSystem::new()
            .with_file(
                "/repo/Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n",
            )
            .with_file("/repo/Cargo.lock", lock(&["demo"]));

        let err =
            read_project_versions(&fs, Path::new("/repo"), &project(&["Cargo.toml"])).unwrap_err();

        assert!(
            matches!(err, AppError::UndeclaredLock { ref lock, .. } if lock == "Cargo.lock"),
            "got {err:?}"
        );
    }

    #[test]
    fn a_declared_lock_is_read_like_any_other_file() {
        let fs = MemoryFileSystem::new()
            .with_file(
                "/repo/Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n",
            )
            .with_file("/repo/Cargo.lock", lock(&["demo"]));

        let versions = read_project_versions(
            &fs,
            Path::new("/repo"),
            &project(&["Cargo.toml", "Cargo.lock"]),
        )
        .unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].path, "Cargo.lock");
        assert_eq!(versions[1].version, "1.2.3".parse().unwrap());
    }

    #[test]
    fn a_shared_lock_is_demanded_even_when_it_holds_other_members() {
        // It records this project's package alongside another's, and vump can
        // now keep that entry in step — so leaving it undeclared is the same
        // stale lock as before.
        let fs = MemoryFileSystem::new()
            .with_file(
                "/repo/Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n",
            )
            .with_file("/repo/Cargo.lock", lock(&["demo", "other"]));

        let err =
            read_project_versions(&fs, Path::new("/repo"), &project(&["Cargo.toml"])).unwrap_err();
        assert!(
            matches!(err, AppError::UndeclaredLock { ref lock, .. } if lock == "Cargo.lock"),
            "got {err:?}"
        );
    }

    #[test]
    fn a_lock_no_manifest_names_is_refused_rather_than_inferred() {
        // Tracking a lock without the manifest that owns it leaves nothing to
        // say which entry the project means.
        let fs = MemoryFileSystem::new().with_file("/repo/Cargo.lock", lock(&["demo"]));

        let err =
            read_project_versions(&fs, Path::new("/repo"), &project(&["Cargo.lock"])).unwrap_err();
        assert!(
            matches!(
                err,
                AppError::VersionFile(VersionFileError::UnidentifiedLock { .. })
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn a_manifest_with_no_lock_beside_it_is_fine() {
        let fs = MemoryFileSystem::new().with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n",
        );

        assert!(read_project_versions(&fs, Path::new("/repo"), &project(&["Cargo.toml"])).is_ok());
    }

    #[test]
    fn only_locks_recording_the_project_version_are_companions() {
        // yarn and pnpm locks carry no version for the project itself, so a
        // bump never invalidates one.
        let paths =
            |declared| companion_locks(declared).map(|(candidates, _)| candidates.join(", "));

        assert_eq!(paths("Cargo.toml").as_deref(), Some("Cargo.lock"));
        assert_eq!(
            paths("ui/package.json").as_deref(),
            Some("ui/package-lock.json, package-lock.json")
        );
        assert!(paths("yarn.lock").is_none());
        assert!(paths("VERSION").is_none());
    }

    #[test]
    fn a_member_looks_upward_for_a_shared_workspace_lock() {
        // A workspace keeps one lock at its root, however deep the member sits.
        let (candidates, _) = companion_locks("crates/api/Cargo.toml").unwrap();
        assert_eq!(
            candidates,
            ["crates/api/Cargo.lock", "crates/Cargo.lock", "Cargo.lock"]
        );
    }

    #[test]
    fn resolves_configured_paths_relative_to_the_config_directory() {
        let resolved = resolve(Path::new("/repo"), "services/api/Cargo.toml");
        assert_eq!(
            resolved,
            Path::new("/repo")
                .join("services")
                .join("api")
                .join("Cargo.toml")
        );
    }
}
