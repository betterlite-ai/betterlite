mod attempt;
mod prompt;
mod review;
mod runner;
mod seat;
mod worktree;

use std::env;
use std::path::{Path, PathBuf};

use crate::development::Development;
use crate::error::GateError;
use crate::facts::resolve_base;
use crate::git::common_dir;
use crate::journal::journal_dir;
use crate::spec::agents::{Agents, Role, Rung, Runner};
use crate::spec::backlog::Status;
use crate::spec::task::{ReviewLevel, Task};
use crate::yaml::read_yaml;

use prompt::Brief;

pub(crate) enum Step {
    Ready(Vec<String>),
    Retry(Vec<String>),
    Reviewed(Vec<String>),
    Climb(Vec<String>),
}

struct Progress {
    feedback: Vec<String>,
    number: u32,
    review_rounds: u32,
}

pub(crate) struct Places<'run> {
    pub(crate) worktree: &'run Path,
    pub(crate) journal: &'run Path,
    pub(crate) git_common_dir: &'run Path,
    pub(crate) home: &'run Path,
}

pub(crate) struct Attempt<'run> {
    pub(crate) agents: &'run Agents,
    pub(crate) brief: &'run Brief<'run>,
    pub(crate) places: &'run Places<'run>,
    pub(crate) number: u32,
}

pub(crate) fn find_runner<'agents>(
    agents: &'agents Agents,
    id: &str,
) -> Result<&'agents Runner, GateError> {
    let found = agents.runners.iter().find(|runner| runner.id == id);
    found.ok_or_else(|| GateError::Dispatch {
        task: String::new(),
        reason: format!("agents.yaml has no runner {id}"),
    })
}

pub(crate) const fn reviewer_role(agents: &Agents, level: ReviewLevel) -> &Role {
    match level {
        ReviewLevel::Standard => &agents.roles.reviewer,
        ReviewLevel::Security => &agents.roles.security_reviewer,
    }
}

fn ready_file<'backlog>(
    development: &'backlog Development,
    task_id: &str,
) -> Result<&'backlog str, GateError> {
    let tasks = &development.backlog.tasks;
    let entry = tasks
        .iter()
        .find(|entry| entry.id == task_id)
        .ok_or_else(|| GateError::UnknownTask(task_id.to_owned()))?;
    if entry.status == Status::Done {
        return Err(GateError::Dispatch {
            task: task_id.to_owned(),
            reason: "the task is already done".to_owned(),
        });
    }
    let open: Vec<&str> = entry
        .depends_on
        .iter()
        .filter(|dependency| {
            tasks
                .iter()
                .any(|other| &other.id == *dependency && other.status != Status::Done)
        })
        .map(String::as_str)
        .collect();
    if !open.is_empty() {
        let reason = format!("dependencies not done: {}", open.join(", "));
        return Err(GateError::Dispatch {
            task: task_id.to_owned(),
            reason,
        });
    }
    entry
        .file
        .as_deref()
        .ok_or_else(|| GateError::NoTaskFile(task_id.to_owned()))
}

fn climb_rung(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    rung: &Rung,
    progress: &mut Progress,
) -> Result<Option<Vec<String>>, GateError> {
    let runner = find_runner(agents, &rung.runner)?;
    for _ in 0..rung.attempts {
        progress.number = progress.number.saturating_add(1);
        let current = Attempt {
            agents,
            brief,
            places,
            number: progress.number,
        };
        match attempt::attempt_once(&current, runner, rung, &progress.feedback)? {
            Step::Ready(lines) => return Ok(Some(lines)),
            Step::Retry(next) => progress.feedback = next,
            Step::Reviewed(next) if progress.review_rounds < agents.policy.max_review_rounds => {
                progress.review_rounds = progress.review_rounds.saturating_add(1);
                progress.feedback = next;
            }
            Step::Reviewed(next) => {
                let reason = format!("review rounds ran out: {}", next.join("; "));
                return Err(GateError::Dispatch {
                    task: brief.task.id.clone(),
                    reason,
                });
            }
            Step::Climb(next) => {
                progress.feedback = next;
                return Ok(None);
            }
        }
    }
    Ok(None)
}

fn run_ladder(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    notes: Vec<String>,
) -> Result<Vec<String>, GateError> {
    let mut progress = Progress {
        feedback: notes,
        number: 0,
        review_rounds: 0,
    };
    for rung in &agents.roles.worker.ladder {
        if let Some(lines) = climb_rung(agents, brief, places, rung, &mut progress)? {
            return Ok(lines);
        }
    }
    let reason = format!(
        "the worker ladder ran out of attempts; last feedback: {}",
        progress.feedback.join("; ")
    );
    Err(GateError::Dispatch {
        task: brief.task.id.clone(),
        reason,
    })
}

pub(crate) fn dispatch(
    development: &Development,
    task_id: &str,
    notes: Vec<String>,
) -> Result<Vec<String>, GateError> {
    let agents: Agents = read_yaml(&development.root.join(".development/agents.yaml"))?;
    let file = ready_file(development, task_id)?;
    let task: Task = development.task(task_id)?;
    let (worktree, branch) = worktree::ensure(&development.root, &agents.worktrees, &task)?;
    let base = resolve_base(&worktree)?;
    let journal = journal_dir(&development.root, task_id)?;
    let brief = Brief {
        task: &task,
        file,
        branch: &branch,
        base: &base,
    };
    let git_common_dir = common_dir(&development.root)?;
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| GateError::Dispatch {
            task: task_id.to_owned(),
            reason: "HOME is not set, so the worker's cache folders are unknown".to_owned(),
        })?;
    let places = Places {
        worktree: &worktree,
        journal: &journal,
        git_common_dir: &git_common_dir,
        home: &home,
    };
    run_ladder(&agents, &brief, &places, notes)
}
