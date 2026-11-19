use std::env::{self, VarError};
use std::fs;
use std::path::Path;

use crate::error::GateError;
use crate::git::git;

pub(crate) struct Facts {
    pub(crate) base: String,
    pub(crate) tree: Tree,
    pub(crate) changed: Vec<String>,
    pub(crate) untracked: Vec<String>,
    pub(crate) numstat: Vec<NumStat>,
    pub(crate) added_rust_lines: Vec<String>,
    pub(crate) added_python_lines: Vec<String>,
    pub(crate) added_litellm_caller_lines: Vec<String>,
}

const LITELLM_CALLERS: [&str; 2] = ["tools/oracle/helpers/*.py", "tools/oracle/sdk_http/*.py"];

pub(crate) struct Tree {
    pub(crate) head: String,
    pub(crate) clean: bool,
}

impl Tree {
    pub(crate) fn read(root: &Path) -> Result<Self, GateError> {
        Ok(Self {
            head: git(root, &["rev-parse", "HEAD"])?.trim().to_owned(),
            clean: git(root, &["status", "--porcelain", "-z"])?.is_empty(),
        })
    }
}

pub(crate) struct NumStat {
    pub(crate) lines: Option<usize>,
    pub(crate) path: String,
}

impl Facts {
    pub(crate) fn collect(root: &Path) -> Result<Self, GateError> {
        let base = resolve_base(root)?;
        let tree = Tree::read(root)?;
        let untracked = untracked_paths(root, &[])?;
        let mut changed: Vec<String> =
            git(root, &["diff", "--name-only", "--no-renames", "-z", &base])?
                .split('\0')
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .chain(untracked.iter().cloned())
                .collect();
        changed.sort_unstable();
        changed.dedup();
        let numstat = git(root, &["diff", "--numstat", "--no-renames", &base])?
            .lines()
            .filter_map(|row| {
                let mut columns = row.splitn(3, '\t');
                let added = columns.next()?.parse::<usize>().ok();
                let deleted = columns.next()?.parse::<usize>().ok();
                let path = columns.next()?.to_owned();
                let lines = added
                    .zip(deleted)
                    .map(|(plus, minus)| plus.saturating_add(minus));
                Some(NumStat { lines, path })
            })
            .collect();
        let excludes = LITELLM_CALLERS.map(|path| format!(":(exclude){path}"));
        let mut python = vec!["*.py"];
        python.extend(excludes.iter().map(String::as_str));
        Ok(Self {
            added_rust_lines: added_lines(root, &base, &["*.rs"])?,
            added_python_lines: added_lines(root, &base, &python)?,
            added_litellm_caller_lines: added_lines(root, &base, &LITELLM_CALLERS)?,
            base,
            tree,
            changed,
            untracked,
            numstat,
        })
    }
}

fn added_lines(root: &Path, base: &str, pathspecs: &[&str]) -> Result<Vec<String>, GateError> {
    let mut arguments = vec![
        "diff",
        "--unified=0",
        "--no-color",
        "--no-renames",
        base,
        "--",
    ];
    arguments.extend_from_slice(pathspecs);
    let mut lines: Vec<String> = git(root, &arguments)?
        .lines()
        .filter(|line| !line.starts_with("+++"))
        .filter_map(|line| line.strip_prefix('+'))
        .map(str::to_owned)
        .collect();
    for path in untracked_paths(root, pathspecs)? {
        let file = root.join(path);
        let text =
            fs::read_to_string(&file).map_err(|source| GateError::Read { path: file, source })?;
        lines.extend(text.lines().map(str::to_owned));
    }
    Ok(lines)
}

fn untracked_paths(root: &Path, pathspecs: &[&str]) -> Result<Vec<String>, GateError> {
    let mut arguments = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
    arguments.extend_from_slice(pathspecs);
    Ok(git(root, &arguments)?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

pub(crate) fn resolve_base(root: &Path) -> Result<String, GateError> {
    match env::var("GATE_BASE") {
        Ok(explicit) if !explicit.is_empty() => {
            let reference = format!("{explicit}^{{commit}}");
            return Ok(git(root, &["rev-parse", "--verify", &reference])?
                .trim()
                .to_owned());
        }
        Ok(_) | Err(VarError::NotPresent) => {}
        Err(VarError::NotUnicode(_)) => return Err(GateError::NotUnicode("GATE_BASE")),
    }
    for candidate in ["origin/main", "main"] {
        let reference = format!("{candidate}^{{commit}}");
        if git(root, &["rev-parse", "--verify", "--quiet", &reference]).is_ok() {
            return Ok(git(root, &["merge-base", "HEAD", candidate])?
                .trim()
                .to_owned());
        }
    }
    Err(GateError::NoBase)
}
