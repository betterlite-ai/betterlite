use std::fs;
use std::path::{Path, PathBuf};

use crate::affected::finish;
use crate::error::GateError;
use crate::git::git;
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
    let path = root.join(worktrees).join(&task.id);
    if !path.join(".git").exists() {
        let location = path.to_str().ok_or_else(|| GateError::Dispatch {
            task: task.id.clone(),
            reason: format!("the worktree path {} is not UTF-8", path.display()),
        })?;
        let reference = format!("refs/heads/{branch}");
        if git(root, &["rev-parse", "--verify", "--quiet", &reference]).is_ok() {
            git(root, &["worktree", "add", location, &branch])?;
        } else {
            git(root, &["worktree", "add", "-b", &branch, location, "main"])?;
        }
    }
    // inside the worker sandbox uv cannot hardlink from its cache and copies the whole LiteLLM venv
    finish(
        "uv",
        &["sync", "--locked", "--quiet"],
        &path.join("tools/oracle/litellm"),
    )?;
    // Claude's sandbox matches writable paths literally, so a path through .. would not cover the worktree
    let canonical = fs::canonicalize(&path).map_err(|source| GateError::Read {
        path: path.clone(),
        source,
    })?;
    Ok((canonical, branch))
}
