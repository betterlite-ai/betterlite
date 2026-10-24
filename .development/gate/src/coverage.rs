mod tests;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

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

pub(crate) struct Tally {
    pub(crate) counts: Vec<usize>,
    pub(crate) uncovered: Vec<String>,
    pub(crate) used: Vec<bool>,
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

pub(crate) fn tally(
    patterns: &[String],
    owners: &[Option<usize>],
    paths: Vec<String>,
    entry_count: usize,
) -> Result<Tally, GateError> {
    let set = glob_set(patterns)?;
    let mut used = vec![false; patterns.len()];
    let mut counts = vec![0_usize; entry_count];
    let mut uncovered = Vec::new();
    for path in paths {
        let indices = set.matches(&path);
        let mut hit_entries = BTreeSet::new();
        for index in &indices {
            if let Some(slot) = used.get_mut(*index) {
                *slot = true;
            }
            if let Some(Some(owner)) = owners.get(*index) {
                hit_entries.insert(*owner);
            }
        }
        for owner in hit_entries {
            if let Some(count) = counts.get_mut(owner) {
                *count = count.saturating_add(1);
            }
        }
        if indices.is_empty() {
            uncovered.push(path);
        }
    }
    Ok(Tally {
        counts,
        uncovered,
        used,
    })
}

fn dead(
    patterns: &[String],
    owners: &[Option<usize>],
    used: &[bool],
    backlog: &Backlog,
) -> Vec<String> {
    let mut dead_globs = Vec::new();
    for ((pattern, owner), hit) in patterns.iter().zip(owners).zip(used) {
        if *hit {
            continue;
        }
        match owner.and_then(|index| backlog.tasks.get(index)) {
            Some(entry) => dead_globs.push(format!("{}: {pattern}", entry.id)),
            None => dead_globs.push(pattern.clone()),
        }
    }
    dead_globs
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
    let mut patterns: Vec<String> = Vec::new();
    let mut owners: Vec<Option<usize>> = Vec::new();
    for exclusion in &litellm.excluded {
        patterns.push(exclusion.glob.clone());
        owners.push(None);
    }
    for (index, entry) in backlog.tasks.iter().enumerate() {
        for glob in &entry.covers {
            patterns.push(glob.clone());
            owners.push(Some(index));
        }
    }
    let Tally {
        counts,
        uncovered,
        used,
    } = tally(&patterns, &owners, paths, backlog.tasks.len())?;
    let dead_globs = dead(&patterns, &owners, &used, backlog);
    let entries = backlog
        .tasks
        .iter()
        .map(|entry| entry.id.clone())
        .zip(counts)
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

pub(crate) fn path_owners(
    folder: &Path,
    litellm: &Litellm,
    backlog: &Backlog,
) -> Result<Vec<String>, GateError> {
    let (_, paths) = read_inventory(folder)?;
    let mut patterns: Vec<String> = litellm
        .excluded
        .iter()
        .map(|exclusion| exclusion.glob.clone())
        .collect();
    let mut owners: Vec<&str> = vec!["excluded"; patterns.len()];
    for entry in &backlog.tasks {
        for glob in &entry.covers {
            patterns.push(glob.clone());
            owners.push(entry.id.as_str());
        }
    }
    let set = glob_set(&patterns)?;
    let mut lines = Vec::new();
    for path in paths {
        let mut names = BTreeSet::new();
        for index in set.matches(&path) {
            if let Some(owner) = owners.get(index) {
                names.insert(*owner);
            }
        }
        let joined = if names.is_empty() {
            "uncovered".to_owned()
        } else {
            names.into_iter().collect::<Vec<_>>().join(",")
        };
        lines.push(format!("{path} {joined}"));
    }
    Ok(lines)
}
