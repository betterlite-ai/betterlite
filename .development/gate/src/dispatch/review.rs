use crate::answer::parse_answer;
use crate::development::Development;
use crate::dispatch::seat::{Seat, invoke};
use crate::dispatch::{Attempt, EarlierReview, Step, find_runner, prompt, reviewer_role};
use crate::error::GateError;
use crate::facts::Facts;
use crate::git::git;
use crate::review_check::review_problems;
use crate::spec::agents::{Agents, Rung, Runner};
use crate::spec::review::{FindingOrigin, Review, ReviewVerdict};
use crate::spec::rules::Rules;
use crate::spec::task::ReviewLevel;
use crate::yaml::read_yaml;

fn reviewer_rung<'agents>(
    agents: &'agents Agents,
    level: ReviewLevel,
    worker_runner: &Runner,
) -> Option<&'agents Rung> {
    let vendor_of = |id: &str| {
        agents
            .runners
            .iter()
            .find(|runner| runner.id == id)
            .map(|runner| runner.vendor)
    };
    let mut ladder = reviewer_role(agents, level).ladder.iter();
    ladder.find(|candidate| {
        !agents.policy.review_by_other_vendor
            || vendor_of(&candidate.runner) != Some(worker_runner.vendor)
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
    let rung = reviewer_rung(attempt.agents, brief.task.review_level, worker_runner)
        .ok_or_else(|| refused("no reviewer rung fits the vendor policy".to_owned()))?;
    let runner = find_runner(attempt.agents, &rung.runner)?;
    let prompt = prompt::reviewer(brief, attempt.earlier);
    let Some(text) = invoke(attempt, runner, rung, Seat::Reviewer, &prompt)? else {
        return Err(refused("the reviewer ended without an answer".to_owned()));
    };
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
