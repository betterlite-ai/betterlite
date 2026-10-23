mod tests;

use std::fs;
use std::path::Path;

use globset::GlobSet;

use crate::builtins::globs::glob_set;
use crate::error::GateError;
use crate::spec::backlog::Backlog;
use crate::spec::litellm::Litellm;

pub(crate) struct Coverage {
    pub(crate) entries: Vec<(String, usize)>,
    pub(crate) uncovered: Vec<String>,
    pub(crate) dead_globs: Vec<String>,
    pub(crate) stale: Option<String>,
}

pub(crate) fn read_inventory(folder: &Path) -> Result<(String, Vec<String>), GateError> {
    let path = folder.join("litellm-inventory.txt");
    let text = fs::read_to_string(&path).map_err(|source| GateError::Read { path, source })?;
    let mut lines = text.lines();
    let commit = lines
        .next()
        .and_then(|line| line.strip_prefix("commit "))
        .unwrap_or_default()
        .to_owned();
    Ok((commit, lines.map(str::to_owned).collect()))
}

fn hits(set: &GlobSet, patterns: &[String], paths: &[String]) -> (Vec<bool>, Vec<String>) {
    let mut used = vec![false; patterns.len()];
    let matched: Vec<bool> = paths
        .iter()
        .map(|path| {
            let indices = set.matches(path);
            for index in &indices {
                if let Some(slot) = used.get_mut(*index) {
                    *slot = true;
                }
            }
            !indices.is_empty()
        })
        .collect();
    let dead = patterns
        .iter()
        .zip(&used)
        .filter(|&(_, hit)| !hit)
        .map(|(pattern, _)| pattern.clone())
        .collect();
    (matched, dead)
}

pub(crate) fn measure(
    folder: &Path,
    litellm: &Litellm,
    backlog: &Backlog,
) -> Result<Coverage, GateError> {
    let (commit, paths) = read_inventory(folder)?;
    let stale = (commit != litellm.commit).then(|| {
        format!(
            "litellm-inventory.txt is for {commit}, litellm.yaml pins {}",
            litellm.commit
        )
    });
    let exclusion_globs: Vec<String> = litellm
        .excluded
        .iter()
        .map(|exclusion| exclusion.glob.clone())
        .collect();
    let (excluded, mut dead_globs) = hits(&glob_set(&exclusion_globs)?, &exclusion_globs, &paths);
    let mut covered = excluded;
    let mut entries = Vec::new();
    for entry in &backlog.tasks {
        let (matched, dead) = hits(&glob_set(&entry.covers)?, &entry.covers, &paths);
        dead_globs.extend(dead.into_iter().map(|glob| format!("{}: {glob}", entry.id)));
        entries.push((entry.id.clone(), matched.iter().filter(|hit| **hit).count()));
        for (slot, hit) in covered.iter_mut().zip(matched) {
            *slot = *slot || hit;
        }
    }
    let uncovered = paths
        .into_iter()
        .zip(covered)
        .filter(|&(_, hit)| !hit)
        .map(|(path, _)| path)
        .collect();
    Ok(Coverage {
        entries,
        uncovered,
        dead_globs,
        stale,
    })
}

pub(crate) fn problems(coverage: &Coverage) -> Vec<String> {
    let mut problems: Vec<String> = coverage.stale.iter().cloned().collect();
    problems.extend(
        coverage
            .dead_globs
            .iter()
            .map(|glob| format!("covers no LiteLLM path: {glob}")),
    );
    if !coverage.uncovered.is_empty() {
        problems.push(format!("{} LiteLLM paths are neither covered by a backlog entry nor excluded; cargo gate coverage lists them", coverage.uncovered.len()));
    }
    problems
}
