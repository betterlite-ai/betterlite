mod tests;

use std::fs::{self, File};
use std::path::Path;
use std::process::ExitStatus;

use crate::checks::Outcome;
use crate::error::GateError;
use crate::git::common_dir;
use crate::process::check_command;
use crate::spec::command::Command;

const HINT_WORDS: [&str; 6] = ["error", "fail", "panicked", "denied", "banned", "mismatch"];

fn hints(output: &str) -> Vec<String> {
    let flagged: Vec<String> = output
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            HINT_WORDS.iter().any(|word| lower.contains(word))
        })
        .take(3)
        .map(|line| line.trim().to_owned())
        .collect();
    if !flagged.is_empty() {
        return flagged;
    }
    let mut last: Vec<String> = output
        .lines()
        .rev()
        .take(3)
        .map(|line| line.trim().to_owned())
        .collect();
    last.reverse();
    last
}

fn run_logged(
    root: &Path,
    log_path: &Path,
    command: &Command,
    environment: &[(&str, String)],
) -> Result<ExitStatus, GateError> {
    let log = File::create(log_path).map_err(|source| GateError::Write {
        path: log_path.to_owned(),
        source,
    })?;
    let errors = log.try_clone().map_err(|source| GateError::Write {
        path: log_path.to_owned(),
        source,
    })?;
    check_command("bash")
        .arg("-c")
        .arg(&command.run)
        .current_dir(root)
        .envs(environment.iter().map(|(key, value)| (*key, value)))
        .stdout(log)
        .stderr(errors)
        .status()
        .map_err(|source| GateError::Spawn {
            program: command.run.clone(),
            source,
        })
}

pub(crate) fn execute(
    root: &Path,
    log_dir: &Path,
    command: &Command,
    environment: &[(&str, String)],
) -> Result<Outcome, GateError> {
    let log_path = log_dir.join(format!("{}.log", command.id));
    let turn = if command.one_at_a_time {
        let path = common_dir(root)?
            .join("betterlite")
            .join(format!("{}.lock", command.id));
        let file = File::create(&path).map_err(|source| GateError::Write {
            path: path.clone(),
            source,
        })?;
        file.lock()
            .map_err(|source| GateError::Lock { path, source })?;
        Some(file)
    } else {
        None
    };
    let status = run_logged(root, &log_path, command, environment)?;
    drop(turn);
    if status.success() {
        return Ok(Outcome::Pass);
    }
    let bytes = fs::read(&log_path).map_err(|source| GateError::Read {
        path: log_path.clone(),
        source,
    })?;
    let output = String::from_utf8(bytes).map_err(|source| GateError::Utf8 {
        what: command.run.clone(),
        source,
    })?;
    let code = status
        .code()
        .map_or_else(|| "a signal".to_owned(), |exit| exit.to_string());
    let mut found = vec![format!("exit {code}, log {}", log_path.display())];
    found.extend(hints(&output));
    Ok(Outcome::Fail {
        why: command.why.clone(),
        hints: found,
    })
}
