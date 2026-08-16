// Seeds the split category directories from the legacy `~/.purple` tree the
// first time a category resolves somewhere else. Runs once at process start,
// before logging opens the log file in the state directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::env::{Category, Paths};

/// Legacy entries per category, relative to `~/.purple`. Askpass retry
/// markers expire within a minute and are not carried over.
fn legacy_entries(category: Category) -> &'static [&'static str] {
    match category {
        Category::Config => &["preferences", "snippets", "providers", "themes"],
        Category::Data => &["certs", "config.original"],
        Category::State => &[
            "history.tsv",
            "recents.json",
            "key_activity.json",
            "snippet_runs.json",
            "sync_history.tsv",
            "purple.log",
            "mcp-audit.log",
        ],
        Category::Cache => &["container_cache.jsonl", "last_version_check"],
    }
}

/// What one startup migration did. Logged by the launcher once the log file
/// is open, since the state directory holding that file may itself be a
/// migration target.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// `(legacy source, new location)` pairs that were copied.
    pub copied: Vec<(PathBuf, PathBuf)>,
    /// `(legacy source, error text)` pairs for copies that failed.
    pub failed: Vec<(PathBuf, String)>,
}

impl MigrationReport {
    pub fn is_empty(&self) -> bool {
        self.copied.is_empty() && self.failed.is_empty()
    }
}

/// Copy legacy files into every category directory that resolved outside
/// `~/.purple` and holds none of that category's files yet. Directories are
/// created with mode 0700; copied files keep their mode. The legacy copies
/// stay in place, so a user can roll back by unsetting the variables.
///
/// A category is seeded all or nothing: when one entry fails to copy, the
/// entries already copied for that category are removed again, so the next
/// start finds the directory untouched and tries again.
pub fn migrate_legacy_layout(paths: &Paths) -> MigrationReport {
    let mut report = MigrationReport::default();
    let legacy = paths.legacy_dir();
    for category in Category::ALL {
        let target = paths.dir(category);
        if target == legacy {
            continue;
        }
        if let Err(e) = ensure_private_dir(target) {
            report
                .failed
                .push((target.to_path_buf(), format!("create directory: {e}")));
            continue;
        }
        let entries = legacy_entries(category);
        if entries.iter().any(|name| target.join(name).exists()) {
            continue;
        }
        let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut failure: Option<(PathBuf, String)> = None;
        for name in entries {
            let src = legacy.join(name);
            if !src.exists() {
                continue;
            }
            let dst = target.join(name);
            match copy_entry(&src, &dst) {
                Ok(()) => done.push((src, dst)),
                Err(e) => {
                    failure = Some((src, e.to_string()));
                    break;
                }
            }
        }
        match failure {
            Some((src, err)) => {
                for (_, dst) in &done {
                    let _ = remove_path(dst);
                }
                report.failed.push((
                    src,
                    format!(
                        "{err} ({} category left unseeded, retried on the next start)",
                        category.name()
                    ),
                ));
            }
            None => report.copied.extend(done),
        }
    }
    report
}

/// Create `dir` (and its parents) when absent, mode 0700 on the leaf. An
/// existing directory is left exactly as it is.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Copy one legacy entry so that `dst` either appears complete or not at
/// all. A file goes through an atomic write. A directory tree is copied
/// into a temporary sibling first and renamed into place at the end; a
/// failure removes the partial copy.
fn copy_entry(src: &Path, dst: &Path) -> io::Result<()> {
    if !src.is_dir() {
        return copy_file(src, dst);
    }
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(dst.file_name().unwrap_or_default());
    tmp_name.push(format!(".purple_tmp.{}", std::process::id()));
    let tmp = dst.with_file_name(tmp_name);
    let _ = fs::remove_dir_all(&tmp);
    let copied = copy_tree(src, &tmp).and_then(|()| fs::rename(&tmp, dst));
    if copied.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    copied
}

/// Copy a directory tree. New directories get mode 0700, files keep theirs.
fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    ensure_private_dir(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            copy_file(&from, &to)?;
        }
    }
    Ok(())
}

/// Copy one file through an atomic write, keeping its mode. Everything that
/// can fail is read before the write, so a returned error means `dst` was
/// never created. Carrying the mode over is best effort: the atomic write
/// leaves 0600 behind, which is never wider than the source.
fn copy_file(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(src)?.permissions().mode() & 0o777
    };
    let content = fs::read(src)?;
    crate::fs_util::atomic_write(dst, &content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(dst, fs::Permissions::from_mode(mode)) {
            log::debug!(
                "[purple] could not carry mode {mode:o} onto {}: {e}",
                dst.display()
            );
        }
    }
    Ok(())
}

/// Remove a copied entry again, file or directory tree.
fn remove_path(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_paths(home: &Path, base: &Path) -> Paths {
        let cfg = base.join("cfg").to_string_lossy().to_string();
        let data = base.join("data").to_string_lossy().to_string();
        let state = base.join("state").to_string_lossy().to_string();
        let cache = base.join("cache").to_string_lossy().to_string();
        Paths::resolve(home, move |k| match k {
            "XDG_CONFIG_HOME" => Some(cfg.clone()),
            "XDG_DATA_HOME" => Some(data.clone()),
            "XDG_STATE_HOME" => Some(state.clone()),
            "XDG_CACHE_HOME" => Some(cache.clone()),
            _ => None,
        })
    }

    fn seed_legacy(home: &Path) -> PathBuf {
        let legacy = home.join(".purple");
        fs::create_dir_all(legacy.join("certs")).unwrap();
        fs::create_dir_all(legacy.join("themes")).unwrap();
        fs::write(legacy.join("preferences"), "theme=Purple\n").unwrap();
        fs::write(legacy.join("providers"), "[aws]\n").unwrap();
        fs::write(legacy.join("snippets"), "").unwrap();
        fs::write(legacy.join("themes/mine.toml"), "name = \"mine\"\n").unwrap();
        fs::write(legacy.join("certs/web-cert.pub"), "cert").unwrap();
        fs::write(legacy.join("config.original"), "Host a\n").unwrap();
        fs::write(legacy.join("history.tsv"), "a\t1\t1\t1\n").unwrap();
        fs::write(legacy.join("recents.json"), "[]").unwrap();
        fs::write(legacy.join("purple.log"), "log\n").unwrap();
        fs::write(legacy.join("container_cache.jsonl"), "{}\n").unwrap();
        fs::write(legacy.join("last_version_check"), "1\n2\n\n").unwrap();
        fs::write(legacy.join(".askpass_a"), "").unwrap();
        fs::write(legacy.join(".DS_Store"), "junk").unwrap();
        legacy
    }

    #[test]
    fn legacy_layout_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        seed_legacy(dir.path());
        let report = migrate_legacy_layout(&Paths::new(dir.path()));
        assert!(report.is_empty(), "{report:?}");
    }

    #[test]
    fn copies_each_category_into_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let legacy = seed_legacy(&home);
        let paths = split_paths(&home, dir.path());

        let report = migrate_legacy_layout(&paths);

        assert!(report.failed.is_empty(), "{report:?}");
        assert_eq!(
            fs::read_to_string(paths.preferences()).unwrap(),
            "theme=Purple\n"
        );
        assert_eq!(
            fs::read_to_string(paths.providers_config()).unwrap(),
            "[aws]\n"
        );
        assert!(paths.snippets_file().exists());
        assert_eq!(
            fs::read_to_string(paths.themes_dir().join("mine.toml")).unwrap(),
            "name = \"mine\"\n"
        );
        assert_eq!(fs::read_to_string(paths.cert_for("web")).unwrap(), "cert");
        assert_eq!(
            fs::read_to_string(paths.config_original()).unwrap(),
            "Host a\n"
        );
        assert!(paths.history().exists());
        assert!(paths.recents().exists());
        assert!(paths.log_file().exists());
        assert!(paths.container_cache().exists());
        assert!(paths.last_version_check().exists());
        // Ephemeral markers and unknown files stay behind.
        assert!(!paths.askpass_marker("a").exists());
        assert!(!paths.state_dir().join(".DS_Store").exists());
        // The legacy tree is untouched.
        assert!(legacy.join("preferences").exists());
        assert!(legacy.join("certs/web-cert.pub").exists());
        assert_eq!(report.copied.len(), 11, "{report:?}");
    }

    #[cfg(unix)]
    #[test]
    fn created_directories_are_private_and_file_modes_survive() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let legacy = seed_legacy(&home);
        fs::set_permissions(legacy.join("providers"), fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(
            legacy.join("preferences"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let paths = split_paths(&home, dir.path());

        migrate_legacy_layout(&paths);

        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for category in Category::ALL {
            assert_eq!(mode(paths.dir(category)), 0o700, "{category:?}");
        }
        assert_eq!(mode(&paths.certs_dir()), 0o700);
        assert_eq!(mode(&paths.themes_dir()), 0o700);
        assert_eq!(mode(&paths.providers_config()), 0o600);
        assert_eq!(mode(&paths.preferences()), 0o644);
    }

    #[test]
    fn a_target_that_already_holds_category_files_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        seed_legacy(&home);
        let paths = split_paths(&home, dir.path());
        fs::create_dir_all(paths.config_dir()).unwrap();
        fs::write(paths.preferences(), "theme=Nord\n").unwrap();

        let report = migrate_legacy_layout(&paths);

        // Config is skipped as a whole: the dotfile-managed preferences stay
        // and the sibling snippets file is not seeded next to it.
        assert_eq!(
            fs::read_to_string(paths.preferences()).unwrap(),
            "theme=Nord\n"
        );
        assert!(!paths.snippets_file().exists());
        // Other categories still migrate.
        assert!(paths.history().exists());
        assert!(paths.cert_for("web").exists());
        assert!(
            !report
                .copied
                .iter()
                .any(|(_, dst)| dst.starts_with(paths.config_dir()))
        );
    }

    #[test]
    fn a_second_run_copies_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        seed_legacy(&home);
        let paths = split_paths(&home, dir.path());

        let first = migrate_legacy_layout(&paths);
        assert!(!first.copied.is_empty());
        let second = migrate_legacy_layout(&paths);
        assert!(second.is_empty(), "{second:?}");
    }

    #[test]
    fn overlapping_categories_each_seed_their_own_files() {
        // Config and state resolve to the same directory. Seeding config must
        // not make the state seed think its files are already there.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        seed_legacy(&home);
        let shared = dir.path().join("shared").to_string_lossy().to_string();
        let paths = Paths::resolve(&home, move |k| match k {
            "PURPLE_CONFIG_DIR" | "PURPLE_STATE_DIR" => Some(shared.clone()),
            _ => None,
        });

        let report = migrate_legacy_layout(&paths);

        assert!(report.failed.is_empty(), "{report:?}");
        assert!(paths.preferences().exists());
        assert!(paths.history().exists());
        assert_eq!(paths.config_dir(), paths.state_dir());
    }

    #[test]
    fn missing_legacy_tree_creates_empty_directories() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let paths = split_paths(&home, dir.path());

        let report = migrate_legacy_layout(&paths);

        assert!(report.is_empty(), "{report:?}");
        for category in Category::ALL {
            assert!(paths.dir(category).is_dir(), "{category:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn ensure_private_dir_leaves_an_existing_directory_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("cfg");
        fs::create_dir_all(&existing).unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&existing).unwrap();
        assert_eq!(
            fs::metadata(&existing).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn a_failed_copy_is_reported_and_does_not_stop_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        seed_legacy(&home);
        let paths = split_paths(&home, dir.path());
        // A file where the cache directory should go makes that category fail.
        fs::create_dir_all(dir.path().join("cache")).unwrap();
        fs::write(dir.path().join("cache/purple"), "not a directory").unwrap();

        let report = migrate_legacy_layout(&paths);

        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert!(paths.preferences().exists());
        assert!(paths.history().exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_mid_category_rolls_back_and_the_next_run_retries() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let legacy = seed_legacy(&home);
        let paths = split_paths(&home, dir.path());
        // history.tsv copies first, then recents.json cannot be read.
        let recents = legacy.join("recents.json");
        fs::set_permissions(&recents, fs::Permissions::from_mode(0o000)).unwrap();

        let first = migrate_legacy_layout(&paths);

        let state_failed = first
            .failed
            .iter()
            .any(|(src, err)| src == &recents && err.contains("state category left unseeded"));
        assert!(state_failed, "{first:?}");
        assert!(
            !paths.history().exists(),
            "the entry copied before the failure is rolled back"
        );
        assert!(
            !first
                .copied
                .iter()
                .any(|(_, dst)| dst.starts_with(paths.state_dir()))
        );
        // The other categories still landed.
        assert!(paths.preferences().exists());
        assert!(paths.cert_for("web").exists());

        fs::set_permissions(&recents, fs::Permissions::from_mode(0o600)).unwrap();
        let second = migrate_legacy_layout(&paths);

        assert!(second.failed.is_empty(), "{second:?}");
        assert!(paths.history().exists());
        assert!(paths.recents().exists());
        assert!(paths.log_file().exists());
        // Only the state category was seeded on the retry.
        assert!(
            second
                .copied
                .iter()
                .all(|(_, dst)| dst.starts_with(paths.state_dir())),
            "{second:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_entry_that_fails_leaves_no_partial_copy() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let legacy = seed_legacy(&home);
        let paths = split_paths(&home, dir.path());
        // One unreadable file inside certs/ fails the whole certs entry.
        fs::write(legacy.join("certs/locked-cert.pub"), "x").unwrap();
        fs::set_permissions(
            legacy.join("certs/locked-cert.pub"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();

        let report = migrate_legacy_layout(&paths);

        assert!(
            report
                .failed
                .iter()
                .any(|(src, _)| src == &legacy.join("certs")),
            "{report:?}"
        );
        assert!(
            !paths.certs_dir().exists(),
            "no half-copied certs directory"
        );
        assert!(
            !paths.config_original().exists(),
            "data category stays unseeded"
        );
        let leftovers: Vec<_> = fs::read_dir(paths.data_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp directory cleaned up: {leftovers:?}"
        );
    }
}
