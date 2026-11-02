use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::development::Development;
use crate::dispatch::progress::Progress;
use crate::dispatch::{overlap, readiness};
use crate::error::GateError;
use crate::git::{common_dir, coordinator};
use crate::journal::journal_dir;
use crate::spec::agents::Agents;
use crate::spec::task::Task;

pub(super) struct State {
    pub(super) journal: PathBuf,
    pub(super) progress: Progress,
    pub(super) lease: File,
}

pub(super) fn state(
    development: &Development,
    agents: &Agents,
    task: &Task,
    notes: Vec<String>,
    grant: u32,
) -> Result<State, GateError> {
    readiness::check(&development.root, task)?;
    let _admission = admission(&coordinator(&development.root)?)?;
    let clashes = overlap::overlaps(development, task)?;
    if !clashes.is_empty() {
        return Err(GateError::Dispatch {
            task: task.id.clone(),
            reason: format!(
                "its scope overlaps work in progress: {}",
                clashes.join("; ")
            ),
        });
    }
    let journal = journal_dir(&development.root, &task.id)?;
    let lease = lock(&journal, &task.id)?;
    let override_blocker = !notes.is_empty() || grant > 0;
    let progress = Progress::load(&journal, notes)?;
    if progress.blocked_reports >= agents.policy.max_blocked_reports && !override_blocker {
        return Err(GateError::Dispatch {
            task: task.id.clone(),
            reason: format!(
                "the repeated blocker still needs an architect's --note explaining the fix: {}",
                progress.feedback.join("; ")
            ),
        });
    }
    Ok(State {
        journal,
        progress,
        lease,
    })
}

fn admission(coordinator: &Path) -> Result<File, GateError> {
    let folder = coordinator.join("betterlite");
    fs::create_dir_all(&folder).map_err(|source| GateError::Write {
        path: folder.clone(),
        source,
    })?;
    let path = folder.join("admission.lock");
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|source| GateError::Write {
            path: path.clone(),
            source,
        })?;
    file.lock()
        .map_err(|source| GateError::Lock { path, source })?;
    Ok(file)
}

pub(super) fn writable(
    agents: &Agents,
    worktree: &Path,
    coordinator: &Path,
    home: &Path,
) -> Result<Vec<String>, GateError> {
    let private = common_dir(worktree)?;
    let (Some(git), Some(home_text)) = (private.to_str(), home.to_str()) else {
        return Err(GateError::NotUnicode("the .git folder or HOME"));
    };
    let mut writable: Vec<String> = agents
        .policy
        .writable
        .iter()
        .map(|entry| {
            entry
                .replace("{git_common_dir}", git)
                .replace("{home}", home_text)
        })
        .collect();
    let shared = coordinator.join("betterlite");
    writable.push(
        shared
            .to_str()
            .ok_or(GateError::NotUnicode("the coordination folder"))?
            .to_owned(),
    );
    if let Some(socket) = env::var_os("GATE_POSTGRES_SOCKET")
        && let Some(directory) = Path::new(&socket).parent()
    {
        writable.push(
            directory
                .to_str()
                .ok_or(GateError::NotUnicode("the PostgreSQL socket"))?
                .to_owned(),
        );
    }
    Ok(writable)
}

fn lock(journal: &Path, task: &str) -> Result<File, GateError> {
    let path = journal.join("dispatch.lock");
    let lease = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|source| GateError::Write {
            path: path.clone(),
            source,
        })?;
    lease.try_lock().map_err(|error| GateError::Dispatch {
        task: task.to_owned(),
        reason: format!("another dispatcher still owns this task: {error}"),
    })?;
    Ok(lease)
}
