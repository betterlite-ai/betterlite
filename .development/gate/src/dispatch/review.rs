use crate::answer::parse_answer;
use crate::development::Development;
use crate::dispatch::seat::{Run, Seat, invoke};
use crate::dispatch::{Attempt, EarlierReview, Step, find_runner, prompt, reviewer_role, vendor};
use crate::error::GateError;
use crate::facts::Facts;
use crate::git::git;
use crate::review_check::review_problems;
use crate::spec::agents::{Agents, Rung, Runner};
use crate::spec::review::{FindingOrigin, Review, ReviewVerdict};
use crate::spec::rules::Rules;
use crate::spec::task::ReviewLevel;
use crate::yaml::read_yaml;

pub(crate) fn reviewer_rungs<'agents>(
    agents: &'agents Agents,
    level: ReviewLevel,
    worker_runner: &Runner,
) -> Vec<&'agents Rung> {
    let ladder = &reviewer_role(agents, level).ladder;
    if !agents.policy.review_by_other_vendor {
        return ladder.iter().collect();
    }
    let on_worker_vendor = |rung: &&Rung| {
        find_runner(agents, &rung.runner).is_ok_and(|runner| runner.vendor == worker_runner.vendor)
    };
    let (same, other): (Vec<&Rung>, Vec<&Rung>) = ladder.iter().partition(on_worker_vendor);
    other.into_iter().chain(same).collect()
}

fn review_text(attempt: &Attempt<'_>, worker_runner: &Runner) -> Result<String, GateError> {
    let brief = attempt.brief;
    let prompt = prompt::reviewer(brief, attempt.earlier);
    for rung in reviewer_rungs(attempt.agents, brief.task.review_level, worker_runner) {
        let runner = find_runner(attempt.agents, &rung.runner)?;
        if vendor::limited(
            attempt.places.git_common_dir,
            attempt.agents,
            &runner.vendor,
        )? {
            continue;
        }
        match invoke(attempt, runner, rung, Seat::Reviewer, &prompt)? {
            Run::Answer(text) => return Ok(text),
            Run::Silent => {
                return Err(GateError::Dispatch {
                    task: brief.task.id.clone(),
                    reason: "the reviewer ended without an answer".to_owned(),
                });
            }
            Run::Limited => {}
        }
    }
    Err(GateError::Dispatch {
        task: brief.task.id.clone(),
        reason: "every runner on the reviewer ladder is at its usage limit".to_owned(),
    })
}

fn step(attempt: &Attempt<'_>, answer: &Review, commit: String) -> Step {
    let brief = attempt.brief;
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
                "{} repeats a LiteLLM bug, for the architect to decide: {line}",
                brief.task.id
            )),
        }
    }
    match answer.verdict {
        ReviewVerdict::Approve => {
            let place = attempt.places.worktree.display();
            let mut lines = vec![format!(
                "{} is ready to merge: branch {} in {place}",
                brief.task.id, brief.branch
            )];
            lines.append(&mut litellm);
            Step::Ready(lines)
        }
        ReviewVerdict::Changes => Step::Reviewed(
            EarlierReview {
                commit,
                findings: ours,
            },
            litellm,
        ),
    }
}

pub(crate) fn review(
    attempt: &Attempt<'_>,
    worker_runner: &Runner,
    worktree: &Development,
) -> Result<Step, GateError> {
    let brief = attempt.brief;
    let refused = |reason: String| GateError::Dispatch {
        task: brief.task.id.clone(),
        reason,
    };
    let text = review_text(attempt, worker_runner)?;
    let answer: Review = parse_answer(&text, "the review")?;
    let rules: Rules = read_yaml(&worktree.root.join(".development/rules.yaml"))?;
    let changed = Facts::collect(&worktree.root)?.changed;
    let problems = review_problems(&answer, &brief.task.id, &rules, &changed);
    if !problems.is_empty() {
        return Err(refused(format!(
            "the review is malformed: {}",
            problems.join("; ")
        )));
    }
    let commit = git(&worktree.root, &["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    Ok(step(attempt, &answer, commit))
}
