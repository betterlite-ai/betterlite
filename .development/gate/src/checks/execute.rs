mod slots;
mod tests;

use std::env;
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::process::ExitStatus;

use crate::checks::Outcome;
use crate::error::GateError;
use crate::git::coordinator;
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
        .env("GATE_BUILD_HELD", "1")
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
        let path = coordinator(root)?
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
    let build = build_slot(root, command)?;
    let status = run_logged(root, &log_path, command, environment)?;
    drop(build);
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

fn build_slot(root: &Path, command: &Command) -> Result<Option<File>, GateError> {
    let configured = env::var("GATE_BUILD_SLOTS")
        .ok()
        .map(|value| value.parse::<u32>())
        .transpose()
        .map_err(|error| {
            GateError::Selection(format!(
                "GATE_BUILD_SLOTS must be a positive integer: {error}"
            ))
        })?;
    if configured == Some(0) {
        return Err(GateError::Selection(
            "GATE_BUILD_SLOTS must be positive".to_owned(),
        ));
    }
    let build = if let Some(count) = configured
        && command.run.contains("cargo ")
        && !command.compiles_nothing
        && env::var("GATE_BUILD_HELD").as_deref() != Ok("1")
    {
        Some(slots::acquire(
            &coordinator(root)?.join("betterlite/builds"),
            count,
            &mut io::stderr(),
        )?)
    } else {
        None
    };
    Ok(build)
}
