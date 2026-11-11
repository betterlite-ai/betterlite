mod tests;

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;

use crate::error::GateError;
use crate::git::git;
use crate::process::check_command;
use crate::spec::report::{MutantTest, Report, ReportStatus};

const SHOWN: usize = 20;

pub(crate) fn mutant_problems(
    root: &Path,
    base: &str,
    report: &Report,
) -> Result<Vec<String>, GateError> {
    if report.status == ReportStatus::Blocked {
        return Ok(Vec::new());
    }
    // the same diff the CI mutants check reads in .development/checks.yaml
    let diff = git(
        root,
        &[
            "diff",
            base,
            "--",
            "*.rs",
            ":(exclude)tools/oracle",
            ":(exclude,glob)**/fuzz/**",
        ],
    )?;
    if diff.trim().is_empty() {
        return Ok(Vec::new());
    }
    let scratch = tempfile::tempdir().map_err(|source| GateError::Write {
        path: env::temp_dir(),
        source,
    })?;
    let diff_path = scratch.path().join("mutants.diff");
    fs::write(&diff_path, diff).map_err(|source| GateError::Write {
        path: diff_path.clone(),
        source,
    })?;
    let output = check_command("cargo")
        .args(["mutants", "--list", "--in-diff"])
        .arg(&diff_path)
        .current_dir(root)
        .output()
        .map_err(|source| GateError::Spawn {
            program: "cargo mutants --list".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(GateError::Failed {
            program: "cargo mutants --list".to_owned(),
            stderr: String::from_utf8(output.stderr)
                .unwrap_or_default()
                .trim()
                .to_owned(),
        });
    }
    let listed = String::from_utf8(output.stdout).map_err(|source| GateError::Utf8 {
        what: "cargo mutants --list".to_owned(),
        source,
    })?;
    let declared = git(
        root,
        &[
            "grep",
            "-h",
            "-o",
            "-E",
            r"\bfn [A-Za-z_][A-Za-z0-9_]*",
            "--",
            "*.rs",
        ],
    )?;
    let functions: BTreeSet<&str> = declared
        .lines()
        .filter_map(|line| line.strip_prefix("fn "))
        .collect();
    let lines: Vec<&str> = listed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    Ok(untested(&lines, &report.mutants, &functions))
}

pub(super) fn untested(
    listed: &[&str],
    entries: &[MutantTest],
    functions: &BTreeSet<&str>,
) -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();
    for line in listed {
        match entries.iter().find(|entry| entry.mutant.trim() == *line) {
            None => problems.push(format!("mutants has no entry for `{line}`")),
            Some(entry)
                if !entry
                    .test
                    .split("::")
                    .any(|segment| functions.contains(segment.trim())) =>
            {
                problems.push(format!(
                    "mutants names test `{}` for `{line}`, and no fn in the Rust sources has a name from that path",
                    entry.test
                ));
            }
            Some(_) => {}
        }
    }
    if problems.is_empty() {
        return problems;
    }
    let total = problems.len();
    problems.truncate(SHOWN);
    if total > SHOWN {
        problems.push(format!(
            "and {} more mutants like these",
            total.saturating_sub(SHOWN)
        ));
    }
    problems.push(
        "List the mutants as worker.md's step 3 says and give each line, exactly as printed, the test that fails on that change; CI's mutants check fails on every one no test catches."
            .to_owned(),
    );
    problems
}
