use crate::answer::parse_answer;
use crate::development::Development;
use crate::dispatch::progress::ReviewerPin;
use crate::dispatch::seat::{Run, Seat, invoke};
use crate::dispatch::{
    Attempt, EarlierReview, Round, Step, find_runner, prompt, reviewer_role, vendor,
};
use crate::error::GateError;
use crate::facts::Facts;
use crate::git::git;
use crate::review_check::review_problems;
use crate::spec::agents::{Agents, Rung, Runner};
use crate::spec::review::{FindingOrigin, Review};
use crate::spec::rules::Rules;
use crate::spec::task::ReviewLevel;
use crate::yaml::read_yaml;

pub(crate) fn reviewer_rungs<'agents>(
    agents: &'agents Agents,
    level: ReviewLevel,
    worker_runner: &Runner,
    pinned: Option<&ReviewerPin>,
) -> Vec<&'agents Rung> {
    let ladder = &reviewer_role(agents, level).ladder;
    // the reviewer of the last round goes first, so a later round reads the code with the same eyes
    let is_pinned = |rung: &&Rung| {
        pinned.is_some_and(|pin| pin.runner == rung.runner && pin.model == rung.model)
    };
    let (first, rest): (Vec<&Rung>, Vec<&Rung>) = ladder.iter().partition(is_pinned);
    if !agents.policy.review_by_other_vendor {
        return first.into_iter().chain(rest).collect();
    }
    let on_worker_vendor = |rung: &&Rung| {
        find_runner(agents, &rung.runner).is_ok_and(|runner| runner.vendor == worker_runner.vendor)
    };
    let (same, other): (Vec<&Rung>, Vec<&Rung>) = rest.into_iter().partition(on_worker_vendor);
    first.into_iter().chain(other).chain(same).collect()
}

fn review_text(
    attempt: &Attempt<'_>,
    worker_runner: &Runner,
    pinned: Option<&ReviewerPin>,
    seat: Seat,
    prompt: &str,
) -> Result<(String, ReviewerPin), GateError> {
    let brief = attempt.brief;
    let level = brief.task.review_level;
    for rung in reviewer_rungs(attempt.agents, level, worker_runner, pinned) {
        let runner = find_runner(attempt.agents, &rung.runner)?;
        if vendor::limited(
            attempt.places.git_common_dir,
            attempt.agents,
            &runner.vendor,
        )? {
            continue;
        }
        let reason = match invoke(attempt, runner, rung, seat, prompt)? {
            Run::Answer { text, .. } => {
                let pin = ReviewerPin {
                    runner: rung.runner.clone(),
                    model: rung.model.clone(),
                };
                return Ok((text, pin));
            }
            Run::Silent => "the reviewer ended without an answer".to_owned(),
            Run::TimedOut => format!(
                "the reviewer hit its {} minute timeout before its answer",
                rung.timeout_minutes
            ),
            Run::Limited => continue,
        };
        return Err(GateError::Dispatch {
            task: brief.task.id.clone(),
            reason,
        });
    }
    Err(GateError::UsageLimit {
        task: brief.task.id.clone(),
        reason: "every runner on the reviewer ladder is at its usage limit".to_owned(),
    })
}

fn round(task_id: &str, answer: &Review, reviewer: ReviewerPin, commit: String) -> Step {
    let mut ours = Vec::new();
    let mut litellm = Vec::new();
    for finding in &answer.findings {
        let line = format!(
            "{} at {}: {} Fix: {}",
            finding.rule, finding.location, finding.problem, finding.fix
        );
        match finding.origin {
            FindingOrigin::Ours => ours.push(line),
            FindingOrigin::Litellm => litellm.push(format!(
                "{task_id} repeats a LiteLLM bug, for the architect to decide: {line}"
            )),
        }
    }
    Step::Reviewed(Round {
        verdict: answer.verdict,
        review: EarlierReview {
            commit,
            findings: ours,
        },
        reviewer,
        litellm,
    })
}

fn checked(
    text: &str,
    task_id: &str,
    rules: &Rules,
    changed: &[String],
) -> Result<Review, Vec<String>> {
    let answer: Review =
        parse_answer(text, "the review").map_err(|error| vec![error.to_string()])?;
    let problems = review_problems(&answer, task_id, rules, changed);
    if problems.is_empty() {
        Ok(answer)
    } else {
        Err(problems)
    }
}

pub(crate) fn review(
    attempt: &Attempt<'_>,
    worker_runner: &Runner,
    worktree: &Development,
) -> Result<Step, GateError> {
    let brief = attempt.brief;
    let task_id = &brief.task.id;
    let rules: Rules = read_yaml(&worktree.root.join(".development/rules.yaml"))?;
    let changed = Facts::collect(&worktree.root)?.changed;
    let commit = git(&worktree.root, &["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    let prompt = prompt::reviewer(brief, attempt.earlier);
    let (first, reviewer) = review_text(
        attempt,
        worker_runner,
        attempt.reviewer,
        Seat::Reviewer,
        &prompt,
    )?;
    let problems = match checked(&first, task_id, &rules, &changed) {
        Ok(answer) => return Ok(round(task_id, &answer, reviewer, commit)),
        Err(found) => found,
    };
    // a malformed answer goes back to the same reviewer once, with what was wrong
    let again = format!(
        "{prompt}\nYour last answer was rejected: {}. Review again and answer with a review that fixes this.\n",
        problems.join("; ")
    );
    let (second, pin) = review_text(
        attempt,
        worker_runner,
        Some(&reviewer),
        Seat::ReviewerAgain,
        &again,
    )?;
    match checked(&second, task_id, &rules, &changed) {
        Ok(answer) => Ok(round(task_id, &answer, pin, commit)),
        Err(still) => Err(GateError::Dispatch {
            task: task_id.clone(),
            reason: format!("the review is malformed: {}", still.join("; ")),
        }),
    }
}
