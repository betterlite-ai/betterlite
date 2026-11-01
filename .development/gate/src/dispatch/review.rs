use std::path::Path;

use crate::answer::parse_answer;
use crate::dispatch::progress::{Panel, ReviewerPin};
use crate::dispatch::seat::{Run, Seat, invoke};
use crate::dispatch::{
    Attempt, EarlierReview, Round, Step, find_runner, prompt, reviewer_role, vendor,
};
use crate::error::GateError;
use crate::facts::Facts;
use crate::git::git;
use crate::review_check::review_problems;
use crate::spec::agents::{Agents, Rung, Runner};
use crate::spec::review::{FindingOrigin, Review, ReviewVerdict};
use crate::spec::rules::Rules;
use crate::spec::task::ReviewLevel;
use crate::yaml::read_yaml;

// the parity pass reads for LiteLLM parity and test strength only, style stays with the standard review and the hooks
pub(crate) const PARITY_RULES: [&str; 4] = [
    "owned-behavior-is-built",
    "errors-match-litellm",
    "red-before-green",
    "bounded-inputs",
];

#[derive(Clone, Copy)]
pub(crate) struct Bench<'pin> {
    pub(crate) panel: Panel,
    pub(crate) pinned: Option<&'pin ReviewerPin>,
}

struct Reading<'run> {
    rules: &'run Rules,
    changed: &'run [String],
}

pub(crate) fn reviewer_rungs<'agents>(
    agents: &'agents Agents,
    level: ReviewLevel,
    bench: Bench<'_>,
    worker_runner: &Runner,
) -> Vec<&'agents Rung> {
    let ladder = &reviewer_role(agents, level, bench.panel).ladder;
    // the reviewer of the last round goes first, so a later round reads the code with the same eyes
    let is_pinned = |rung: &&Rung| {
        bench.pinned.is_some_and(|pin| {
            pin.panel == bench.panel && pin.runner == rung.runner && pin.model == rung.model
        })
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
    bench: Bench<'_>,
    seat: Seat,
    prompt: &str,
) -> Result<(String, ReviewerPin), GateError> {
    let brief = attempt.brief;
    let level = brief.task.review_level;
    for rung in reviewer_rungs(attempt.agents, level, bench, worker_runner) {
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
                    panel: bench.panel,
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

fn round(
    task_id: &str,
    answers: &[&Review],
    verdict: ReviewVerdict,
    reviewer: ReviewerPin,
    commit: String,
) -> Step {
    let mut ours = Vec::new();
    let mut litellm = Vec::new();
    for finding in answers.iter().flat_map(|answer| &answer.findings) {
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
        verdict,
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
    reading: &Reading<'_>,
    panel: Panel,
) -> Result<Review, Vec<String>> {
    let answer: Review =
        parse_answer(text, "the review").map_err(|error| vec![error.to_string()])?;
    let mut problems = review_problems(&answer, task_id, reading.rules, reading.changed);
    if panel == Panel::Parity {
        for finding in &answer.findings {
            if !PARITY_RULES.contains(&finding.rule.as_str()) {
                problems.push(format!(
                    "the finding at {} cites {}, and the parity review reports only {}",
                    finding.location,
                    finding.rule,
                    PARITY_RULES.join(", ")
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(answer)
    } else {
        Err(problems)
    }
}

fn panel_review(
    attempt: &Attempt<'_>,
    worker_runner: &Runner,
    reading: &Reading<'_>,
    bench: Bench<'_>,
    earlier: Option<&EarlierReview>,
) -> Result<(Review, ReviewerPin), GateError> {
    let brief = attempt.brief;
    let task_id = &brief.task.id;
    let role = reviewer_role(attempt.agents, brief.task.review_level, bench.panel);
    let prompt = prompt::reviewer(brief, &role.instructions, earlier);
    let (seat, seat_again) = match bench.panel {
        Panel::Standard => (Seat::Reviewer, Seat::ReviewerAgain),
        Panel::Parity => (Seat::ParityReviewer, Seat::ParityReviewerAgain),
    };
    let (first, reviewer) = review_text(attempt, worker_runner, bench, seat, &prompt)?;
    let problems = match checked(&first, task_id, reading, bench.panel) {
        Ok(answer) => return Ok((answer, reviewer)),
        Err(found) => found,
    };
    // a malformed answer goes back to the same reviewer once, with what was wrong
    let again = format!(
        "{prompt}\nYour last answer was rejected: {}. Review again and answer with a review that fixes this.\n",
        problems.join("; ")
    );
    let same = Bench {
        panel: bench.panel,
        pinned: Some(&reviewer),
    };
    let (second, pin) = review_text(attempt, worker_runner, same, seat_again, &again)?;
    match checked(&second, task_id, reading, bench.panel) {
        Ok(answer) => Ok((answer, pin)),
        Err(still) => Err(GateError::Dispatch {
            task: task_id.clone(),
            reason: format!("the review is malformed: {}", still.join("; ")),
        }),
    }
}

pub(crate) fn review(
    attempt: &Attempt<'_>,
    worker_runner: &Runner,
    root: &Path,
) -> Result<Step, GateError> {
    let task = attempt.brief.task;
    let rules: Rules = read_yaml(&root.join(".development/rules.yaml"))?;
    let changed = Facts::collect(root)?.changed;
    let reading = Reading {
        rules: &rules,
        changed: &changed,
    };
    let commit = git(root, &["rev-parse", "HEAD"])?.trim().to_owned();
    let bench = Bench {
        panel: attempt.reviewer.map_or(Panel::Standard, |pin| pin.panel),
        pinned: attempt.reviewer,
    };
    let (answer, reviewer) =
        panel_review(attempt, worker_runner, &reading, bench, attempt.earlier)?;
    // the first approval of a standard task that ports referenced code gets one parity pass, and later rounds stay with it
    let parity_due = bench.panel == Panel::Standard
        && answer.verdict == ReviewVerdict::Approve
        && task.review_level == ReviewLevel::Standard
        && !task.references.is_empty();
    if !parity_due {
        return Ok(round(
            &task.id,
            &[&answer],
            answer.verdict,
            reviewer,
            commit,
        ));
    }
    let parity = Bench {
        panel: Panel::Parity,
        pinned: None,
    };
    let (second, pin) = panel_review(attempt, worker_runner, &reading, parity, None)?;
    Ok(round(
        &task.id,
        &[&answer, &second],
        second.verdict,
        pin,
        commit,
    ))
}
