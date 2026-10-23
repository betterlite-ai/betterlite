mod tests;

use std::collections::BTreeSet;

use crate::spec::report::{OptionVerdict, PriorArt, Report, ReportStatus};

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

fn shape_problems(report: &Report, task_id: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if report.task != task_id {
        problems.push(format!(
            "the report is for {}, the branch is {task_id}",
            report.task
        ));
    }
    let consistent = match report.status {
        ReportStatus::Done => !report.summary.trim().is_empty() && report.blocker.trim().is_empty(),
        ReportStatus::Blocked => !report.blocker.trim().is_empty(),
    };
    if !consistent {
        problems.push(
            "a done report needs a summary and an empty blocker; a blocked report needs a blocker"
                .to_owned(),
        );
    }
    let link_ok = |link: &str| link.is_empty() || link.starts_with("https://");
    if report.deviations.iter().any(|deviation| {
        deviation.what.trim().is_empty()
            || deviation.why.trim().is_empty()
            || !link_ok(&deviation.link)
    }) {
        problems.push("a deviation needs what, why, and an https link or an empty one".to_owned());
    }
    if report.notes.iter().any(|note| note.trim().is_empty()) {
        problems.push("notes hold no empty strings".to_owned());
    }
    problems
}

pub(crate) fn report_problems(
    report: &Report,
    task_id: &str,
    added_dependencies: &BTreeSet<String>,
) -> Vec<String> {
    let mut problems = shape_problems(report, task_id);
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
