mod tests;

use std::collections::BTreeSet;
use std::fs;

use crate::coverage;
use crate::development::Development;
use crate::error::GateError;
use crate::git::{common_dir, git};
use crate::spec::backlog::{BacklogEntry, Resolution, Status};
use crate::spec::litellm::Litellm;
use crate::spec::task::ReviewLevel;
use crate::yaml::read_yaml;

pub(crate) fn components(paths: &[String]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for path in paths {
        if let Some(rest) = path.strip_prefix("crates/")
            && let Some((name, _)) = rest.split_once('/')
        {
            names.insert(name.to_owned());
        }
    }
    names.into_iter().collect()
}

fn state(entry: &BacklogEntry) -> String {
    let status = match entry.status {
        Status::Todo => "todo",
        Status::InProgress => "in progress",
        Status::Done => "done",
        Status::Blocked => "blocked",
    };
    let resolution = match entry.resolution {
        Some(Resolution::Merged) => ", merged",
        Some(Resolution::Obsolete) => ", obsolete",
        Some(Resolution::Duplicate) => ", duplicate",
        Some(Resolution::Superseded) => ", superseded",
        None => "",
    };
    let pr = entry
        .pr
        .map(|number| format!(", PR #{number}"))
        .unwrap_or_default();
    format!("status: {status}{resolution}{pr}")
}

fn links(development: &Development, entry: &BacklogEntry) -> Vec<String> {
    let status_of = |id: &str| {
        development
            .backlog
            .tasks
            .iter()
            .find(|other| other.id == id)
            .map_or_else(
                || "missing".to_owned(),
                |other| state(other).replace("status: ", ""),
            )
    };
    let mut lines = Vec::new();
    for (label, link) in [
        ("parent", &entry.parent),
        ("split from", &entry.split_from),
        ("superseded by", &entry.superseded_by),
    ] {
        if let Some(id) = link {
            lines.push(format!("{label}: {id}"));
        }
    }
    if let Some(reason) = &entry.blocked_reason {
        lines.push(format!("blocked: {reason}"));
    }
    let depends: Vec<String> = entry
        .depends_on
        .iter()
        .map(|id| format!("{id} ({})", status_of(id)))
        .collect();
    lines.push(format!("depends on: {}", depends.join(", ")));
    lines.push(format!(
        "ready once this is done: {}",
        unblocked(development, entry).join(", ")
    ));
    lines
}

fn unblocked(development: &Development, entry: &BacklogEntry) -> Vec<String> {
    let mut unblocks = Vec::new();
    for other in &development.backlog.tasks {
        if !other.depends_on.contains(&entry.id) || other.status != Status::Todo {
            continue;
        }
        let rest_done = other
            .depends_on
            .iter()
            .filter(|id| **id != entry.id)
            .all(|id| {
                development
                    .backlog
                    .tasks
                    .iter()
                    .any(|task| &task.id == id && task.status == Status::Done)
            });
        if rest_done {
            unblocks.push(other.id.clone());
        }
    }
    unblocks
}

fn history(development: &Development, entry: &BacklogEntry) -> Result<Vec<String>, GateError> {
    let root = &development.root;
    let entry_file = format!(".development/backlog/{}.yaml", entry.id);
    let task_file = entry
        .file
        .as_ref()
        .map(|file| format!(".development/{file}"));
    let mut tracked = vec![entry_file.as_str()];
    if let Some(file) = &task_file {
        tracked.push(file.as_str());
    }
    let mut args = vec!["log", "--format=%cs", "--"];
    args.extend(tracked);
    let dates = git(root, &args)?;
    let mut lines = Vec::new();
    if let (Some(updated), Some(created)) = (dates.lines().next(), dates.lines().last()) {
        lines.push(format!("created: {created}, updated: {updated}"));
    }
    if let Some(number) = entry.pr {
        let grep = format!("--grep=(#{number})$");
        let merged = git(root, &["log", "-1", "--format=%H %cs", &grep])?;
        if let Some((commit, date)) = merged.trim().split_once(' ') {
            let size = git(root, &["show", "--shortstat", "--format=", commit])?;
            lines.push(format!("resolved: {date}, {}", size.trim()));
        }
    }
    let journal = common_dir(root)?.join("betterlite").join(&entry.id);
    if let Ok(listing) = fs::read_dir(&journal) {
        let attempts = listing
            .filter_map(Result::ok)
            .filter(|item| {
                item.file_name().to_str().is_some_and(|name| {
                    name.starts_with("attempt-") && name.ends_with("-worker.log")
                })
            })
            .count();
        lines.push(format!("worker attempts in the local journal: {attempts}"));
    }
    Ok(lines)
}

pub(crate) fn show(development: &Development, id: &str) -> Result<(Vec<String>, bool), GateError> {
    let entry = development
        .backlog
        .tasks
        .iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| GateError::UnknownTask(id.to_owned()))?;
    let mut lines = vec![
        format!("{} {}", entry.id, entry.title),
        state(entry),
        format!("milestone: {}", entry.milestone),
    ];
    lines.extend(links(development, entry));
    match &entry.file {
        Some(file) => {
            let task = development.task(id)?;
            lines.push(format!(
                "task: .development/{file} (review {}, budget {} lines)",
                match task.review_level {
                    ReviewLevel::Security => "security",
                    ReviewLevel::Standard => "standard",
                },
                task.budget.max_changed_lines
            ));
            lines.push(format!(
                "components: {}",
                components(&task.scope.paths).join(", ")
            ));
        }
        None => lines.push("task: not written yet".to_owned()),
    }
    let folder = development.root.join(".development");
    let litellm: Litellm = read_yaml(&folder.join("litellm.yaml"))?;
    let measured = coverage::measure(&folder, &litellm, &development.backlog)?;
    if let Some((_, count)) = measured.entries.iter().find(|(entry_id, _)| entry_id == id) {
        lines.push(format!("covers: {count} LiteLLM paths"));
    }
    lines.extend(history(development, entry)?);
    Ok((lines, true))
}
