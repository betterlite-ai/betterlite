mod tests;

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::GateError;
use crate::process::check_command;
use crate::spec::catalog::Catalog;
use crate::spec::command::Command;
use crate::spec::task::Task;
use crate::yaml::read_yaml;

// the gate and CI set these for every command, so shellcheck sees them as assigned
const HEADER: &str = "declare -x GATE_ALL_TESTS GATE_BASE GATE_BUILD_SLOTS GATE_LOG_DIR GATE_ORACLE_CASES GATE_POSTGRES_SOCKET GATE_TASK GITHUB_ACTIONS\n";

pub(crate) struct Script {
    pub(crate) origin: String,
    pub(crate) run: String,
}

pub(crate) fn scripts(path: &Path) -> Result<Vec<Script>, GateError> {
    let commands: Vec<Command> = if path.file_name().is_some_and(|name| name == "checks.yaml") {
        let catalog: Catalog = read_yaml(path)?;
        catalog
            .base
            .commands
            .into_iter()
            .chain(catalog.optional)
            .collect()
    } else {
        let task: Task = read_yaml(path)?;
        task.checks.commands
    };
    Ok(commands
        .into_iter()
        .map(|command| Script {
            origin: format!("{} command {}", path.display(), command.id),
            run: command.run,
        })
        .collect())
}

pub(crate) fn origin_of(line: &str, files: &[(PathBuf, String)]) -> String {
    for (file, origin) in files {
        let prefix = format!("{}:", file.display());
        if let Some(rest) = line.strip_prefix(&prefix)
            && let Some((digits, after)) = rest.split_once(':')
            && let Ok(row) = digits.parse::<usize>()
        {
            return format!("{origin}:{}:{after}", row.saturating_sub(1));
        }
    }
    line.to_owned()
}

fn write_scripts(
    folder: &Path,
    index: usize,
    path: &Path,
) -> Result<Vec<(PathBuf, String)>, GateError> {
    let mut files = Vec::new();
    for script in scripts(path)? {
        let file = folder.join(format!("{index}-{}.sh", files.len()));
        fs::write(&file, format!("{HEADER}{}", script.run)).map_err(|source| GateError::Write {
            path: file.clone(),
            source,
        })?;
        files.push((file, script.origin));
    }
    Ok(files)
}

pub(crate) fn shellcheck(root: &Path, paths: &[PathBuf]) -> Result<(Vec<String>, bool), GateError> {
    let folder = tempfile::tempdir().map_err(|source| GateError::Write {
        path: PathBuf::from("a temporary folder for shellcheck"),
        source,
    })?;
    let mut lines = Vec::new();
    let mut passed = true;
    for (index, path) in paths.iter().enumerate() {
        let files = write_scripts(folder.path(), index, path)?;
        if files.is_empty() {
            continue;
        }
        // checks.yaml gets every optional check like the .sh files; a new task file gets the default ones, since its commands are short test lines the planner writes
        let checks = if path.file_name().is_some_and(|name| name == "checks.yaml") {
            format!("--rcfile={}", root.join(".shellcheckrc").display())
        } else {
            "--norc".to_owned()
        };
        let output = check_command("shellcheck")
            .arg("--shell=bash")
            .arg(checks)
            .arg("--format=gcc")
            .args(files.iter().map(|(file, _)| file))
            .current_dir(root)
            .output()
            .map_err(|source| GateError::Spawn {
                program: "shellcheck".to_owned(),
                source,
            })?;
        passed &= output.status.success();
        for stream in [output.stdout, output.stderr] {
            let text = String::from_utf8(stream).map_err(|source| GateError::Utf8 {
                what: "shellcheck".to_owned(),
                source,
            })?;
            lines.extend(text.lines().map(|line| origin_of(line, &files)));
        }
    }
    Ok((lines, passed))
}
