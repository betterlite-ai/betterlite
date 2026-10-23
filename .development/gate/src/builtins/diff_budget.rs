use std::fs;
use std::str;

use crate::builtins::Context;
use crate::builtins::globs::glob_set;
use crate::error::GateError;

pub(crate) fn check(context: &Context<'_>) -> Result<Vec<String>, GateError> {
    let excludes = glob_set(&context.development.catalog.settings.budget_excludes)?;
    let tracked: usize = context
        .facts
        .numstat
        .iter()
        .filter(|row| !excludes.is_match(&row.path))
        .filter_map(|row| row.lines)
        .sum();
    let mut total = tracked;
    for path in context
        .facts
        .untracked
        .iter()
        .filter(|path| !excludes.is_match(path))
    {
        let file = context.development.root.join(path);
        let bytes = fs::read(&file).map_err(|source| GateError::Read { path: file, source })?;
        let lines = str::from_utf8(&bytes).map_or(0, |text| text.lines().count());
        total = total.saturating_add(lines);
    }
    let limit = context.task.budget.max_changed_lines;
    if total <= limit {
        return Ok(Vec::new());
    }
    Ok(vec![format!(
        "{total} changed lines, budget is {limit}; split the work or report blocked with the count"
    )])
}
