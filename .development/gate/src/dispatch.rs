mod attempt;
mod chain;
mod checkpoint;
mod maintenance;
pub(crate) mod overlap;
mod preparation;
mod progress;
mod prompt;
mod readiness;
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
use crate::git::coordinator;
use crate::spec::agents::{Agents, Role, Runner};
use crate::spec::backlog::Status;
use crate::spec::review::ReviewVerdict;
use crate::spec::task::{ReviewLevel, Task};
use crate::yaml::read_yaml;

use chain::{Chain, Link};
use progress::{NextRun, Panel, Progress, Red, ReviewerPin, WorkerSession};
use prompt::Brief;
use serde::{Deserialize, Serialize};

pub(crate) struct Round {
    pub(crate) verdict: ReviewVerdict,
    pub(crate) review: EarlierReview,
    pub(crate) reviewer: ReviewerPin,
    pub(crate) litellm: Vec<String>,
}

pub(crate) struct Failure {
    pub(crate) feedback: Vec<String>,
    pub(crate) red: Vec<Red>,
}

// Fix goes on with the same work in a fix turn, Retry starts fresh, Blocked goes back to the same session once
pub(crate) enum Step {
    Reviewed(Round),
    Fix(Failure),
    Retry(Failure),
    Blocked(Vec<String>),
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

pub(crate) fn availability(development: &Development) -> Result<(), GateError> {
    let git = coordinator(&development.root)?;
    maintenance::check(&git, "")?;
    let agents: Agents = read_yaml(&development.root.join(".development/agents.yaml"))?;
    vendor::available(&git, &agents, "")
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

const BLOCKED_NOTE: &str = "Unfinished work is not a blocker, so go on until the task's checks pass. A gate command that prints a 'gate: waiting for' line is queued behind other builds, so poll it until it ends and never answer while one runs. Answer blocked again only for a scope, budget or outside blocker, with its evidence: the command, output, file and line.";

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
            "{} blocked reports in a row; last feedback: {}",
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
) -> Result<ControlFlow<Vec<String>>, GateError> {
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
            Ok(ControlFlow::Break(lines))
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
    chain: &Chain<'_>,
    brief: &Brief<'_>,
    places: &Places<'_>,
    progress: &mut Progress,
    step: Step,
) -> Result<ControlFlow<Vec<String>>, GateError> {
    let agents = chain.agents;
    match step {
        Step::Reviewed(round) => return apply_round(agents, brief, places, progress, round),
        Step::Fix(failure) if progress.fix_turns < agents.policy.max_fix_turns => {
            progress.feedback = failure.feedback;
            progress.fix_turns = progress.fix_turns.saturating_add(1);
            progress.next = NextRun::Fix;
        }
        Step::Fix(failure) | Step::Retry(failure) => {
            progress.fix_turns = 0;
            progress.next = NextRun::Fresh;
            // the same red at the end of two starts in a row moves the task to another vendor
            let stuck = failure.red.iter().any(|red| progress.red.contains(red));
            progress.feedback = failure.feedback;
            progress.red = failure.red;
            progress.rung = chain.next_rung(progress, stuck);
        }
        Step::Blocked(next) => {
            // set before the count, so the architect's dispatch after a second report starts fresh
            progress.fix_turns = 0;
            progress.next = NextRun::Fresh;
            note_blocked(agents, brief, progress, next)?;
            progress.feedback.push(BLOCKED_NOTE.to_owned());
            progress.next = NextRun::Fix;
        }
    }
    Ok(ControlFlow::Continue(()))
}

fn run_once(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    pick: &Link<'_>,
    progress: &mut Progress,
) -> Result<Option<Step>, GateError> {
    maintenance::check(places.git_common_dir, &brief.task.id)?;
    let interrupted = checkpoint::restore(places.journal, progress, pick.runner, pick.rung)?;
    // saved after the increment, so a killed dispatch never writes over this run's journal files
    progress.number = progress.number.saturating_add(1);
    progress.save(places.journal)?;
    let fix = progress.next == NextRun::Fix && !progress.feedback.is_empty();
    let resume = progress
        .session
        .as_ref()
        .filter(|session| {
            (fix || interrupted)
                && pick.runner.resume.is_some()
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
    if turn.session.is_some() || turn.step.is_some() {
        progress.session = turn.session.map(|id| WorkerSession {
            runner: pick.runner.id.clone(),
            model: pick.rung.model.clone(),
            id,
        });
    }
    Ok(turn.step)
}

fn next_run<'chain, 'agents>(
    chain: &'chain Chain<'agents>,
    brief: &Brief<'_>,
    places: &Places<'_>,
    progress: &mut Progress,
) -> Result<&'chain Link<'agents>, GateError> {
    let agents = chain.agents;
    let fresh = progress.next == NextRun::Fresh;
    // a rung used up before this dispatch, or by a smaller ladder, hands over as if its last start had failed
    if fresh && !progress.open(&agents.roles.worker.ladder, progress.rung) {
        progress.rung = chain.next_rung(progress, false);
    }
    // granted starts raise the cap too
    let most = agents
        .policy
        .max_worker_attempts
        .saturating_add(progress.granted);
    let current = chain.links.get(progress.rung);
    let Some(own) = current.filter(|_| !fresh || progress.used < most) else {
        let reason = format!(
            "the worker ladder ran out of attempts; last feedback: {}",
            progress.feedback.join("; ")
        );
        return Err(GateError::Dispatch {
            task: brief.task.id.clone(),
            reason,
        });
    };
    progress.limited = false;
    if let Some(pick) = chain.pick(places, progress, own)? {
        return Ok(pick);
    }
    progress.limited = true;
    let reason = format!(
        "every runner left on the worker ladder is unavailable ({}); last feedback: {}",
        vendor::causes(places.git_common_dir, agents)?,
        progress.feedback.join("; ")
    );
    Err(GateError::Unavailable {
        task: brief.task.id.clone(),
        reason,
    })
}

fn run_ladder(
    agents: &Agents,
    brief: &Brief<'_>,
    places: &Places<'_>,
    progress: &mut Progress,
    grant: u32,
) -> Result<Vec<String>, GateError> {
    let chain = Chain::new(agents)?;
    progress.fit(&agents.roles.worker.ladder);
    progress.grant(&agents.roles.worker.ladder, grant);
    loop {
        let fresh = progress.next == NextRun::Fresh;
        let pick = next_run(&chain, brief, places, progress)?;
        let outcome = run_once(agents, brief, places, pick, progress)?;
        let removed = attempt::remove_extra_targets(places.worktree)?;
        // a run that hit a usage limit marked its vendor, so the next pick goes around it
        let Some(step) = outcome else {
            continue;
        };
        // only a fresh start counts, never a fix turn or a review fix, and it moves the task to the rung it ran on
        if fresh {
            progress.rung = pick.index;
            if let Some(started) = progress.starts.get_mut(pick.index) {
                *started = started.saturating_add(1);
            }
            progress.used = progress.used.saturating_add(1);
        }
        let interrupted = matches!(step, Step::Retry(_)) && progress.session.is_some();
        let flow = apply_step(&chain, brief, places, progress, step)?;
        progress.feedback.extend(removed);
        checkpoint::save_turn(places.journal, progress, interrupted)?;
        if let ControlFlow::Break(lines) = flow {
            return Ok(lines);
        }
    }
}

pub(crate) fn dispatch(
    development: &Development,
    task_id: &str,
    notes: Vec<String>,
    grant: u32,
) -> Result<Vec<String>, GateError> {
    maintenance::check(&coordinator(&development.root)?, task_id)?;
    let agents: Agents = read_yaml(&development.root.join(".development/agents.yaml"))?;
    vendor::available(&coordinator(&development.root)?, &agents, task_id)?;
    let file = ready_file(development, task_id)?;
    let task: Task = development.task(task_id)?;
    let preparation::State {
        journal,
        mut progress,
        lease: _lease,
    } = preparation::state(development, &agents, &task, notes, grant)?;
    let (worktree, branch) = worktree::ensure(&development.root, &agents.worktrees, &task)?;
    let base = resolve_base(&worktree)?;
    let brief = Brief {
        task: &task,
        file,
        branch: &branch,
        base: &base,
    };
    let git_common_dir = coordinator(&development.root)?;
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| GateError::Dispatch {
            task: task_id.to_owned(),
            reason: "HOME is not set, so the worker's cache folders are unknown".to_owned(),
        })?;
    let writable = preparation::writable(&agents, &worktree, &git_common_dir, &home)?;
    let places = Places {
        worktree: &worktree,
        journal: &journal,
        git_common_dir: &git_common_dir,
        home: &home,
        writable: &writable,
    };
    let outcome = run_ladder(&agents, &brief, &places, &mut progress, grant);
    progress.save(&journal).and(outcome)
}
