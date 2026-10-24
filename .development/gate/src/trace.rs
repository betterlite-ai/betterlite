mod tests;

use crate::builtins::globs::glob_set;
use crate::development::Development;
use crate::deviations::read_log;
use crate::error::GateError;
use crate::spec::backlog::{BacklogEntry, Status};
use crate::spec::task::Task;

pub(crate) struct PinnedRange<'url> {
    pub(crate) path: &'url str,
    pub(crate) lines: Option<(u32, u32)>,
}

pub(crate) fn pinned_range(url: &str) -> Option<PinnedRange<'_>> {
    let (_, after_blob) = url.split_once("/blob/")?;
    let (_, located) = after_blob.split_once('/')?;
    let Some((path, fragment)) = located.split_once("#L") else {
        return Some(PinnedRange {
            path: located,
            lines: None,
        });
    };
    let (first, last) = fragment.split_once("-L").unwrap_or((fragment, fragment));
    let lines = first.parse().ok().zip(last.parse().ok());
    Some(PinnedRange { path, lines })
}

pub(crate) fn split_line(target: &str) -> (&str, Option<u32>) {
    match target.rsplit_once(':') {
        Some((path, line))
            if !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (path, line.parse().ok())
        }
        _ => (target, None),
    }
}

fn label(entry: &BacklogEntry) -> String {
    let status = match entry.status {
        Status::Todo => "todo",
        Status::InProgress => "in progress",
        Status::Done => "done",
        Status::Blocked => "blocked",
    };
    format!("{} ({status}) {}", entry.id, entry.title)
}

fn ported<'task>(
    task: &'task Task,
    path: &str,
    line: Option<u32>,
    covers: bool,
) -> Vec<&'task str> {
    let mut urls = Vec::new();
    for reference in &task.references {
        let Some(range) = pinned_range(&reference.url) else {
            continue;
        };
        let in_lines = match (line, range.lines) {
            (Some(wanted), Some((first, last))) => (first..=last).contains(&wanted),
            (Some(_), None) | (None, _) => true,
        };
        if (!covers || range.path == path) && in_lines {
            urls.push(reference.url.as_str());
        }
    }
    urls
}

fn entry_lines(
    development: &Development,
    entry: &BacklogEntry,
    path: &str,
    line: Option<u32>,
) -> Result<Vec<String>, GateError> {
    let covers = glob_set(&entry.covers)?.is_match(path);
    let Some(file) = &entry.file else {
        return Ok(if covers {
            vec![format!(
                "{path} is covered by {}, not planned yet",
                label(entry)
            )]
        } else {
            Vec::new()
        });
    };
    let task = development.task(&entry.id)?;
    let writes = glob_set(&task.scope.paths)?.is_match(path);
    let urls = ported(&task, path, line, covers);
    if (!covers && !writes) || (covers && line.is_some() && urls.is_empty()) {
        return Ok(Vec::new());
    }
    let role = if covers { "covered" } else { "written" };
    let mut lines = vec![format!(
        "{path} is {role} by {} in .development/{file}",
        label(entry)
    )];
    lines.extend(urls.iter().map(|url| format!("  ports {url}")));
    if covers {
        lines.push(format!("  writes {}", task.scope.paths.join(", ")));
    }
    if let Some(log) = read_log(&development.root, &entry.id)? {
        lines.extend(
            log.deviations
                .iter()
                .map(|deviation| format!("  deviates: {} ({})", deviation.what, deviation.why)),
        );
    }
    Ok(lines)
}

pub(crate) fn trace(
    development: &Development,
    target: &str,
) -> Result<(Vec<String>, bool), GateError> {
    let (path, line) = split_line(target);
    let mut lines = Vec::new();
    for entry in &development.backlog.tasks {
        lines.extend(entry_lines(development, entry, path, line)?);
    }
    let found = !lines.is_empty();
    if !found {
        lines.push(format!("nothing in the backlog covers or writes {target}"));
    }
    Ok((lines, found))
}
