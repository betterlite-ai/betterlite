mod attempt;
mod overlap;
mod progress;
mod prompt;
mod review;
mod runner;
mod sandbox;
mod seat;
mod tests;
mod vendor;
mod worktree;

use std::env;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use crate::development::Development;
use crate::error::GateError;
use crate::facts::resolve_base;
use crate::git::common_dir;
use crate::journal::journal_dir;
use crate::spec::agents::{Agents, Role, Rung, Runner};
use crate::spec::backlog::Status;
use crate::spec::review::ReviewVerdict;
use crate::spec::task::{ReviewLevel, Task};
use crate::yaml::read_yaml;

use progress::{NextRun, Panel, Progress, ReviewerPin, WorkerSession};
use prompt::Brief;
use seat::Seat;
use serde::{Deserialize, Serialize};

pub(crate) struct Round {
    pub(crate) verdict: ReviewVerdict,
    pub(crate) review: EarlierReview,
    pub(crate) reviewer: ReviewerPin,
    pub(crate) litellm: Vec<String>,
}

// Fix goes on with the same work in a fix turn, Retry starts fresh on the same rung, Climb moves up a rung
pub(crate) enum Step {
    Reviewed(Round),
    Fix(Vec<String>),
    Retry(Vec<String>),
    Climb(Vec<String>),
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EarlierReview {
    pub(crate) commit: String,
    pub(crate) findings: Vec<String>,
}

pub(crate) struct Places<'run> {
    pub(crate) worktree: &'run Path,
    pub(crate) journal: &'run Path,
    pub(crate) git_common_dir: &'run Path,
    pub(crate) home: &'run Path,
    pub(crate) writable: &'run [String],
}

pub(crate) struct Attempt<'run> {
    pub(crate) agents: &'run Agents,
    pub(crate) brief: &'run Brief<'run>,
    pub(crate) places: &'run Places<'run>,
    pub(crate) number: u32,
    pub(crate) earlier: Option<&'run EarlierReview>,
    pub(crate) reviewer: Option<&'run ReviewerPin>,
    pub(crate) resume: Option<&'run str>,
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

fn writable_dirs(
    agents: &Agents,
    git_common_dir: &Path,
    home: &Path,
) -> Result<Vec<String>, GateError> {
    let (Some(git), Some(home_text)) = (git_common_dir.to_str(), home.to_str()) else {
        return Err(GateError::NotUnicode("the .git folder or HOME"));
    };
    Ok(agents
        .policy
        .writable
        .iter()
        .map(|entry| {
            entry
                .replace("{git_common_dir}", git)
                .replace("{home}", home_text)
        })
        .collect())
}

pub(crate) const fn reviewer_role(agents: &Agents, level: ReviewLevel, panel: Panel) -> &Role {
    match (panel, level) {
        (Panel::Parity, _) => &agents.roles.parity_reviewer,
        (Panel::Standard, ReviewLevel::Standard) => &agents.roles.reviewer,
        (Panel::Standard, ReviewLevel::Security) => &agents.roles.security_reviewer,
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

fn note_blocked(
    agents: &Agents,
    brief: &Brief<'_>,
    progress: &mut Progress,
    next: Vec<String>,
) -> Result<(), GateError> {
    progress.blocked_reports = if progress.last_blocked.saturating_add(1) == progress.number {
        progress.blocked_reports.saturating_add(1)
    } else {
        1
    };
    progress.last_blocked = progress.number;
    progress.feedback = next;
    if progress.blocked_reports >= agents.policy.max_blocked_reports {
        let reason = format!(
            "{} workers in a row reported blocked; last feedback: {}",
            progress.blocked_reports,
            progress.feedback.join("; ")
        );
        return Err(GateError::Dispatch {
            task: brief.task.id.clone(),
            reason,
        });
    }
    Ok(())
}

fn apply_round(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    progress: &mut Progress,
    mut round: Round,
) -> Result<ControlFlow<Option<Vec<String>>>, GateError> {
    progress.litellm.append(&mut round.litellm);
    // the parity pass gets max_review_rounds of its own, so a long standard review still leaves it rounds
    if progress
        .reviewer
        .as_ref()
        .is_some_and(|pin| pin.panel != round.reviewer.panel)
    {
        progress.review_rounds = 0;
    }
    progress.reviewer = Some(round.reviewer);
    progress.next = NextRun::Fix;
    progress.fix_turns = 0;
    match round.verdict {
        ReviewVerdict::Approve => {
            progress.feedback.clear();
            progress.earlier = Some(round.review);
            let mut lines = vec![format!(
                "{} is ready to merge: branch {} in {}",
                brief.task.id,
                brief.branch,
                places.worktree.display()
            )];
            lines.append(&mut progress.litellm);
            Ok(ControlFlow::Break(Some(lines)))
        }
        ReviewVerdict::Changes => {
            progress.feedback.clone_from(&round.review.findings);
            progress.earlier = Some(round.review);
            if progress.review_rounds >= agents.policy.max_review_rounds {
                let reason = format!("review rounds ran out: {}", progress.feedback.join("; "));
                return Err(GateError::Dispatch {
                    task: brief.task.id.clone(),
                    reason,
                });
            }
            progress.review_rounds = progress.review_rounds.saturating_add(1);
            Ok(ControlFlow::Continue(()))
        }
    }
}

fn apply_step(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    progress: &mut Progress,
    step: Step,
) -> Result<ControlFlow<Option<Vec<String>>>, GateError> {
    match step {
        Step::Reviewed(round) => return apply_round(agents, brief, places, progress, round),
        Step::Fix(next) => {
            progress.feedback = next;
            if progress.fix_turns < agents.policy.max_fix_turns {
                progress.fix_turns = progress.fix_turns.saturating_add(1);
                progress.next = NextRun::Fix;
            } else {
                progress.fix_turns = 0;
                progress.next = NextRun::Fresh;
            }
        }
        Step::Retry(next) => {
            progress.feedback = next;
            progress.fix_turns = 0;
            progress.next = NextRun::Fresh;
        }
        Step::Climb(next) => {
            progress.fix_turns = 0;
            progress.next = NextRun::Fresh;
            note_blocked(agents, brief, progress, next)?;
            return Ok(ControlFlow::Break(None));
        }
    }
    Ok(ControlFlow::Continue(()))
}

struct Pick<'agents> {
    runner: &'agents Runner,
    rung: &'agents Rung,
    counted: bool,
}

fn other_vendor_rungs<'agents>(
    agents: &'agents Agents,
    places: &Places<'_>,
    current: usize,
    runner: &Runner,
) -> Result<Vec<(&'agents Runner, &'agents Rung)>, GateError> {
    let mut found = Vec::new();
    for rung in agents
        .roles
        .worker
        .ladder
        .iter()
        .skip(current.saturating_add(1))
    {
        let other = find_runner(agents, &rung.runner)?;
        if other.vendor != runner.vendor
            && !vendor::limited(places.git_common_dir, agents, &other.vendor)?
        {
            found.push((other, rung));
        }
    }
    Ok(found)
}

fn overflow_rung<'agents>(
    agents: &'agents Agents,
    places: &Places<'_>,
    current: usize,
    runner: &Runner,
) -> Result<Option<(&'agents Runner, &'agents Rung)>, GateError> {
    let git = places.git_common_dir;
    let own = vendor::find_vendor(agents, &runner.vendor)?;
    if !agents.policy.overflow || vendor::seat_free(git, agents, own, Seat::Worker)? {
        return Ok(None);
    }
    for (other, rung) in other_vendor_rungs(agents, places, current, runner)? {
        let account = vendor::find_vendor(agents, &other.vendor)?;
        if vendor::seat_free(git, agents, account, Seat::Worker)? {
            return Ok(Some((other, rung)));
        }
    }
    Ok(None)
}

fn pick_rung<'agents>(
    agents: &'agents Agents,
    places: &Places<'_>,
    current: usize,
    rung: &'agents Rung,
) -> Result<Option<Pick<'agents>>, GateError> {
    let own = find_runner(agents, &rung.runner)?;
    if vendor::limited(places.git_common_dir, agents, &own.vendor)? {
        // the try goes to a later rung of another vendor, so this rung keeps its attempts for when its vendor is back
        let later = other_vendor_rungs(agents, places, current, own)?;
        return Ok(later.first().map(|&(runner, lent)| Pick {
            runner,
            rung: lent,
            counted: false,
        }));
    }
    let (runner, chosen) = overflow_rung(agents, places, current, own)?.unwrap_or((own, rung));
    Ok(Some(Pick {
        runner,
        rung: chosen,
        counted: true,
    }))
}

fn run_once(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    pick: &Pick<'_>,
    progress: &mut Progress,
) -> Result<Option<Step>, GateError> {
    // saved after the increment, so a killed dispatch never writes over this run's journal files
    progress.number = progress.number.saturating_add(1);
    progress.save(places.journal)?;
    let fix = progress.next == NextRun::Fix && !progress.feedback.is_empty();
    let resume = progress
        .session
        .as_ref()
        .filter(|session| {
            fix && pick.runner.resume.is_some()
                && session.runner == pick.runner.id
                && session.model == pick.rung.model
        })
        .map(|session| session.id.as_str());
    let current = Attempt {
        agents,
        brief,
        places,
        number: progress.number,
        earlier: progress.earlier.as_ref(),
        reviewer: progress.reviewer.as_ref(),
        resume,
    };
    let turn = attempt::attempt_once(&current, pick.runner, pick.rung, &progress.feedback)?;
    // a run stopped by a usage limit keeps the session of the run before it
    if turn.step.is_some() {
        progress.session = turn.session.map(|id| WorkerSession {
            runner: pick.runner.id.clone(),
            model: pick.rung.model.clone(),
            id,
        });
    }
    Ok(turn.step)
}

fn climb_rung<'agents>(
    agents: &'agents Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    rung: &'agents Rung,
    progress: &mut Progress,
) -> Result<Option<Vec<String>>, GateError> {
    let last = progress.rung.saturating_add(1) == agents.roles.worker.ladder.len();
    // granted starts raise both caps, on the last rung only
    let extra = if last { progress.granted } else { 0 };
    let attempts = rung.attempts.saturating_add(extra);
    let most = agents.policy.max_worker_attempts.saturating_add(extra);
    loop {
        let fresh = progress.next == NextRun::Fresh;
        if fresh && (progress.rung_used >= attempts || progress.used >= most) {
            return Ok(None);
        }
        let Some(pick) = pick_rung(agents, places, progress.rung, rung)? else {
            progress.limited = true;
            return Ok(None);
        };
        let outcome = run_once(agents, brief, places, &pick, progress)?;
        let removed = attempt::remove_extra_targets(places.worktree)?;
        // a run that hit a usage limit marked its vendor, so the next pick goes around it
        let Some(step) = outcome else {
            continue;
        };
        // only a fresh start counts on the ladder, never a fix turn or a review fix
        if fresh {
            if pick.counted {
                progress.rung_used = progress.rung_used.saturating_add(1);
            }
            progress.used = progress.used.saturating_add(1);
        }
        let flow = apply_step(agents, brief, places, progress, step)?;
        progress.feedback.extend(removed);
        if let ControlFlow::Break(lines) = flow {
            return Ok(lines);
        }
    }
}

fn run_ladder(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    progress: &mut Progress,
    grant: u32,
) -> Result<Vec<String>, GateError> {
    progress.grant(&agents.roles.worker.ladder, grant);
    while let Some(rung) = agents.roles.worker.ladder.get(progress.rung) {
        progress.limited = false;
        if let Some(lines) = climb_rung(agents, brief, places, rung, progress)? {
            return Ok(lines);
        }
        if progress.limited {
            let reason = format!(
                "every runner left on the worker ladder is at its usage limit; last feedback: {}",
                progress.feedback.join("; ")
            );
            return Err(GateError::UsageLimit {
                task: brief.task.id.clone(),
                reason,
            });
        }
        progress.rung = progress.rung.saturating_add(1);
        progress.rung_used = 0;
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
    grant: u32,
) -> Result<Vec<String>, GateError> {
    let agents: Agents = read_yaml(&development.root.join(".development/agents.yaml"))?;
    let file = ready_file(development, task_id)?;
    let task: Task = development.task(task_id)?;
    let clashes = overlap::overlaps(development, &task)?;
    if !clashes.is_empty() {
        return Err(GateError::Dispatch {
            task: task_id.to_owned(),
            reason: format!(
                "its scope overlaps work in progress: {}",
                clashes.join("; ")
            ),
        });
    }
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
        writable: &writable_dirs(&agents, &git_common_dir, &home)?,
    };
    let mut progress = Progress::load(&journal, notes)?;
    let outcome = run_ladder(&agents, &brief, &places, &mut progress, grant);
    progress.save(&journal).and(outcome)
}
