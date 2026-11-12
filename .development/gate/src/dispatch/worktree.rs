mod tests;

use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::affected::finish;
use crate::error::GateError;
use crate::git::{common_dir, git};
use crate::spec::task::Task;

pub(crate) fn branch_name(task: &Task) -> String {
    let mut slug = String::new();
    for character in task.title.chars() {
        let next = if character.is_ascii_alphanumeric() {
            Some(character.to_ascii_lowercase())
        } else {
            (!slug.ends_with('-') && !slug.is_empty()).then_some('-')
        };
        slug.extend(next);
    }
    let trimmed: String = slug.trim_end_matches('-').chars().take(40).collect();
    format!("task/{}-{}", task.id, trimmed.trim_end_matches('-'))
}

pub(crate) fn ensure(
    root: &Path,
    worktrees: &str,
    task: &Task,
) -> Result<(PathBuf, String), GateError> {
    let branch = branch_name(task);
    let folder = root.join(worktrees);
    let path = folder.join(&task.id);
    let new = !path.join(".git").exists();
    prepare(root, &path, &branch)?;
    if new {
        seed(&folder.join("clippy-seed"), &path)?;
    }
    // inside the worker sandbox uv cannot hardlink from its cache and copies the whole LiteLLM venv
    finish(
        "uv",
        &["sync", "--locked", "--quiet"],
        &path.join("tools/oracle/litellm"),
    )?;
    // the alias builds the gate quietly, and workers took that silent first cargo gate for a hang and quit
    finish(
        "cargo",
        &[
            "build",
            "--quiet",
            "--locked",
            "--manifest-path",
            ".development/gate/Cargo.toml",
        ],
        &path,
    )?;
    // Claude's sandbox matches writable paths literally, so a path through .. would not cover the worktree
    let canonical = fs::canonicalize(&path).map_err(|source| GateError::Read {
        path: path.clone(),
        source,
    })?;
    Ok((canonical, branch))
}

fn refuse(target: &Path, reason: &str) -> GateError {
    GateError::Dispatch {
        task: target.display().to_string(),
        reason: reason.to_owned(),
    }
}

fn clone(root: &Path, target: &Path) -> Result<(), GateError> {
    let source = root
        .to_str()
        .ok_or(GateError::NotUnicode("the checkout source"))?;
    let location = target
        .to_str()
        .ok_or(GateError::NotUnicode("the task checkout"))?;
    git(
        root,
        &[
            "clone",
            "--quiet",
            "--local",
            "--no-checkout",
            source,
            location,
        ],
    )?;
    Ok(())
}

fn configure(root: &Path, target: &Path) -> Result<(), GateError> {
    let coordinator = common_dir(root)?;
    let directory = coordinator
        .to_str()
        .ok_or(GateError::NotUnicode("the coordination folder"))?;
    let remote = git(root, &["remote", "get-url", "origin"])?;
    git(target, &["remote", "set-url", "origin", remote.trim()])?;
    git(target, &["config", "betterlite.coordinator", directory])?;
    git(
        target,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    )?;
    // commit and fetch start a detached gc, and its reflog expire holds HEAD.lock while the next git command needs it
    git(target, &["config", "maintenance.auto", "false"])?;
    for key in [
        "user.name",
        "user.email",
        "commit.gpgsign",
        "core.hooksPath",
    ] {
        if let Ok(value) = git(root, &["config", "--get", key]) {
            git(target, &["config", key, value.trim()])?;
        }
    }
    for name in ["pre-commit", "pre-push"] {
        let hook = coordinator.join("hooks").join(name);
        if hook.is_file() {
            let destination = target.join(".git/hooks").join(name);
            fs::copy(&hook, &destination).map_err(|source| GateError::Write {
                path: destination,
                source,
            })?;
        }
    }
    Ok(())
}

fn prepare(root: &Path, target: &Path, branch: &str) -> Result<(), GateError> {
    let canonical = fs::canonicalize(root).map_err(|source| GateError::Read {
        path: root.to_owned(),
        source,
    })?;
    let dotgit = target.join(".git");
    if dotgit.is_symlink() {
        return Err(refuse(target, "refusing a symlinked .git"));
    }
    if dotgit.exists() {
        if git(target, &["branch", "--show-current"])?.trim() != branch {
            return Err(refuse(target, "checkout is on a different task branch"));
        }
        if dotgit.is_file() {
            migrate(&canonical, target)?;
        }
    } else {
        clone(&canonical, target)?;
        let reference = format!("refs/heads/{branch}");
        let base = if git(&canonical, &["show-ref", "--verify", "--quiet", &reference]).is_ok() {
            format!("origin/{branch}")
        } else {
            "origin/main".to_owned()
        };
        git(target, &["switch", "--no-track", "-c", branch, &base])?;
    }
    configure(&canonical, target)
}

// cargo rebuilds a unit when a source is newer than its output, so copied units stay fresh only under older sources
fn seed(store: &Path, checkout: &Path) -> Result<(), GateError> {
    let Some(commit) = copy_seed(store, checkout)? else {
        return Ok(());
    };
    // 2000 rather than 1970, since zip and wheel builds refuse dates before 1980
    let old = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_hours(262_968))
        .ok_or_else(|| refuse(checkout, "the year 2000 is not a valid file time"))?;
    for name in git(checkout, &["ls-files", "-z"])?.split_terminator('\0') {
        stamp(&checkout.join(name), old)?;
    }
    let changed = ["diff", "--name-only", "-z", commit.trim(), "HEAD"];
    let now = SystemTime::now();
    for name in git(checkout, &changed)?.split_terminator('\0') {
        stamp(&checkout.join(name), now)?;
    }
    git(checkout, &["update-index", "-q", "--refresh"])?;
    Ok(())
}

fn copy_seed(store: &Path, checkout: &Path) -> Result<Option<String>, GateError> {
    let recorded = store.join("commit");
    if !recorded.is_file() {
        return Ok(None);
    }
    let lock = store.join("lock");
    let shared = File::open(&lock).map_err(|source| GateError::Read {
        path: lock.clone(),
        source,
    })?;
    shared
        .lock_shared()
        .map_err(|source| GateError::Lock { path: lock, source })?;
    let commit = fs::read_to_string(&recorded).map_err(|source| GateError::Read {
        path: recorded,
        source,
    })?;
    let snapshot = store.join("target");
    let copy = checkout.join("target");
    finish(
        "cp",
        &[
            "-a",
            snapshot
                .to_str()
                .ok_or(GateError::NotUnicode("the clippy seed"))?,
            copy.to_str()
                .ok_or(GateError::NotUnicode("the task checkout"))?,
        ],
        checkout,
    )?;
    Ok(Some(commit))
}

fn stamp(path: &Path, time: SystemTime) -> Result<(), GateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(GateError::Read {
                path: path.to_owned(),
                source,
            });
        }
    }
    File::open(path)
        .and_then(|file| file.set_modified(time))
        .map_err(|source| GateError::Write {
            path: path.to_owned(),
            source,
        })
}

fn idle(target: &Path) -> Result<(), GateError> {
    let canonical = fs::canonicalize(target).map_err(|source| GateError::Read {
        path: target.to_owned(),
        source,
    })?;
    let processes = Path::new("/proc");
    let failed = |source| GateError::Read {
        path: processes.to_owned(),
        source,
    };
    for entry in fs::read_dir(processes).map_err(failed)? {
        let process = entry.map_err(failed)?.path();
        if let Ok(cwd) = fs::read_link(process.join("cwd"))
            && cwd.starts_with(&canonical)
        {
            return Err(refuse(
                target,
                &format!(
                    "checkout is still used by {}; stop it before migration",
                    process.display()
                ),
            ));
        }
    }
    Ok(())
}

fn linked(root: &Path, target: &Path) -> Result<PathBuf, GateError> {
    idle(target)?;
    let directory = git(target, &["rev-parse", "--absolute-git-dir"])?;
    let old = PathBuf::from(directory.trim());
    if old.parent() != Some(common_dir(root)?.join("worktrees").as_path()) {
        return Err(refuse(
            target,
            "linked checkout does not belong to this coordinator",
        ));
    }
    let operations = [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "BISECT_LOG",
    ];
    let failed = |source| GateError::Read {
        path: old.clone(),
        source,
    };
    let mut locked = false;
    for entry in fs::read_dir(&old).map_err(failed)? {
        locked |= entry
            .map_err(failed)?
            .path()
            .extension()
            .is_some_and(|extension| extension == "lock");
    }
    if locked || operations.iter().any(|name| old.join(name).exists()) {
        return Err(refuse(
            target,
            "checkout has an unfinished Git operation; recover it before migration",
        ));
    }
    Ok(old)
}

fn migrate(root: &Path, target: &Path) -> Result<(), GateError> {
    let old = linked(root, target)?;
    let head = git(target, &["rev-parse", "HEAD"])?;
    let branch = git(target, &["symbolic-ref", "HEAD"])?;
    let name = target
        .file_name()
        .ok_or_else(|| refuse(target, "checkout has no directory name"))?;
    let parent = target
        .parent()
        .ok_or_else(|| refuse(target, "checkout has no parent directory"))?;
    let backup = common_dir(root)?.join("betterlite/isolation").join(name);
    if backup.join("gitfile").exists() || backup.join("linked-metadata").exists() {
        return Err(refuse(
            target,
            "a migration backup already exists; inspect it before retrying",
        ));
    }
    let failed = |source| GateError::Write {
        path: target.to_owned(),
        source,
    };
    fs::create_dir_all(&backup).map_err(failed)?;
    let scratch = tempfile::Builder::new()
        .prefix(".git-isolation-")
        .tempdir_in(parent)
        .map_err(failed)?;
    let copied = scratch.path().join("checkout");
    clone(root, &copied)?;
    git(&copied, &["update-ref", branch.trim(), head.trim()])?;
    git(&copied, &["symbolic-ref", "HEAD", branch.trim()])?;
    configure(root, &copied)?;
    fs::copy(old.join("index"), copied.join(".git/index")).map_err(failed)?;
    preserve_info(&common_dir(root)?, &copied.join(".git/info"), target)?;
    replace(target, &copied, &old, &backup)
}

fn preserve_info(common: &Path, info: &Path, target: &Path) -> Result<(), GateError> {
    let failed = |source| GateError::Write {
        path: target.to_owned(),
        source,
    };
    fs::create_dir_all(info).map_err(failed)?;
    for name in ["exclude", "attributes"] {
        let metadata = common.join("info").join(name);
        let destination = info.join(name);
        if metadata.is_file() {
            fs::copy(metadata, destination).map_err(failed)?;
        } else {
            match fs::remove_file(destination) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(failed(error)),
            }
        }
    }
    Ok(())
}

fn replace(target: &Path, copied: &Path, old: &Path, backup: &Path) -> Result<(), GateError> {
    let status = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
    let before = git(target, &status)?;
    let head = git(target, &["rev-parse", "HEAD"])?;
    let dotgit = target.join(".git");
    let failed = |source| GateError::Write {
        path: target.to_owned(),
        source,
    };
    fs::rename(&dotgit, backup.join("gitfile")).map_err(failed)?;
    let outcome = (|| {
        fs::rename(copied.join(".git"), &dotgit).map_err(failed)?;
        if git(target, &status)? != before || git(target, &["rev-parse", "HEAD"])? != head {
            return Err(refuse(
                target,
                "migration changed HEAD or index/worktree status",
            ));
        }
        fs::rename(old, backup.join("linked-metadata")).map_err(failed)?;
        Ok(())
    })();
    if outcome.is_err() {
        if dotgit.is_dir() {
            fs::rename(&dotgit, copied.join(".git")).map_err(failed)?;
        }
        fs::rename(backup.join("gitfile"), &dotgit).map_err(failed)?;
    }
    outcome
}
