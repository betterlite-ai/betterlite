use std::fs;
use std::path::{Path, PathBuf};

use crate::affected::finish;
use crate::error::GateError;
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
    let script_path = root.join(".development/ops/isolate.py");
    let source_path = root
        .to_str()
        .ok_or(GateError::NotUnicode("the checkout source"))?;
    let target = path
        .to_str()
        .ok_or(GateError::NotUnicode("the task checkout"))?;
    let script = script_path
        .to_str()
        .ok_or(GateError::NotUnicode("the isolation script"))?;
    finish(
        "uv",
        &[
            "run",
            "--locked",
            "--project",
            ".development/ops",
            "python",
            script,
            source_path,
            target,
            &branch,
        ],
        root,
    )?;
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
