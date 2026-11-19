pub(crate) mod mutants;
mod tests;

use std::collections::BTreeSet;
use std::fs;
use std::mem;
use std::path::Path;

use crate::answer::parse_answer;
use crate::error::GateError;
use crate::spec::report::{KeptPart, OptionVerdict, PriorArt, Report, ReportStatus};
use crate::spec::task::Task;

fn prior_art_problems(entry: &PriorArt) -> Vec<String> {
    let mut problems = Vec::new();
    let chosen = entry
        .options
        .iter()
        .filter(|option| option.verdict == OptionVerdict::Chosen)
        .count();
    if entry.need.trim().is_empty() || entry.options.len() < 2 || chosen != 1 {
        problems.push(format!(
            "prior_art '{}' needs a need, at least two compared options and exactly one chosen",
            entry.need
        ));
    }
    let unsupported = entry
        .options
        .iter()
        .filter(|option| !option.link.starts_with("https://") || option.why.trim().is_empty());
    problems.extend(unsupported.map(|option| {
        format!(
            "prior_art option '{}' needs an https link and a why",
            option.name
        )
    }));
    problems
}

fn owned_problems(report: &Report, task: &Task) -> Vec<String> {
    let mut problems = Vec::new();
    let owns_litellm = task.references.iter().any(|reference| {
        reference
            .url
            .starts_with("https://github.com/BerriAI/litellm/blob/")
    });
    if report.status == ReportStatus::Done && owns_litellm && report.owned.is_empty() {
        problems.push(
            "owned lists each behavior a user can observe in the LiteLLM code the references own"
                .to_owned(),
        );
    }
    let unsupported = report.owned.iter().filter(|item| {
        item.behavior.trim().is_empty()
            || !item.litellm.starts_with("https://")
            || item.code.trim().is_empty()
            || item.test.trim().is_empty()
    });
    problems.extend(unsupported.map(|item| {
        format!(
            "owned '{}' needs an https link to the LiteLLM line, the code that does it and the test that proves it",
            item.behavior
        )
    }));
    problems
}

fn shape_problems(report: &Report, task_id: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if report.task != task_id {
        problems.push(format!(
            "the report is for {}, the branch is {task_id}",
            report.task
        ));
    }
    let consistent = match report.status {
        ReportStatus::Done => {
            report.summary.trim().chars().count() >= 40 && report.blocker.trim().is_empty()
        }
        ReportStatus::Blocked => !report.blocker.trim().is_empty(),
    };
    if !consistent {
        problems.push(
            "a done report needs a summary of at least 40 characters and an empty blocker; a blocked report needs a blocker"
                .to_owned(),
        );
    }
    if report.notes.iter().any(|note| note.trim().is_empty()) {
        problems.push("notes hold no empty strings".to_owned());
    }
    if !report.kept.is_empty() {
        problems.push(
            "kept names parts to copy, but this branch has no earlier done answer to copy them from; give those lists in full"
                .to_owned(),
        );
    }
    problems
}

pub(crate) fn report_problems(
    report: &Report,
    task: &Task,
    added_dependencies: &BTreeSet<String>,
) -> Vec<String> {
    let mut problems = shape_problems(report, &task.id);
    problems.extend(owned_problems(report, task));
    problems.extend(report.prior_art.iter().flat_map(prior_art_problems));
    let chosen: BTreeSet<String> = report
        .prior_art
        .iter()
        .flat_map(|entry| &entry.options)
        .filter(|option| option.verdict == OptionVerdict::Chosen)
        .map(|option| option.name.to_ascii_lowercase())
        .collect();
    let unexplained = added_dependencies
        .iter()
        .filter(|dependency| !chosen.contains(&dependency.to_ascii_lowercase()));
    problems.extend(unexplained.map(|dependency| {
        format!(
            "new dependency {dependency} has no prior_art entry that chose it over an alternative"
        )
    }));
    problems
}

pub(crate) fn with_kept(
    mut report: Report,
    journal: &Path,
    number: u32,
) -> Result<Report, GateError> {
    let earlier = (1..number)
        .rev()
        .map(|previous| journal.join(format!("attempt-{previous}-worker.report.json")))
        .find(|path| path.exists());
    if let Some(path) = earlier.filter(|_| !report.kept.is_empty()) {
        let text = fs::read_to_string(&path).map_err(|source| GateError::Read {
            path: path.clone(),
            source,
        })?;
        let mut last: Report = parse_answer(&text, "the saved report")?;
        for part in mem::take(&mut report.kept) {
            match part {
                KeptPart::Owned => report.owned = mem::take(&mut last.owned),
                KeptPart::PriorArt => report.prior_art = mem::take(&mut last.prior_art),
                KeptPart::Mutants => report.mutants = mem::take(&mut last.mutants),
            }
        }
    }
    if report.kept.is_empty() {
        let path = journal.join(format!("attempt-{number}-worker.report.json"));
        let text = serde_json::to_string(&report).map_err(|source| GateError::Json {
            what: "the report".to_owned(),
            source,
        })?;
        fs::write(&path, text).map_err(|source| GateError::Write { path, source })?;
    }
    Ok(report)
}
