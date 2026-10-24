mod tests;

use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::process::{Child, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::error::GateError;
use crate::process::check_command;
use crate::spec::agents::{OutputMode, PLACEHOLDERS, Rung, Runner};

pub(crate) struct Invocation<'paths> {
    pub(crate) worktree: &'paths Path,
    pub(crate) git_common_dir: &'paths Path,
    pub(crate) home: &'paths Path,
    pub(crate) sandbox_settings: &'paths Path,
    pub(crate) writable_roots: &'paths str,
    pub(crate) schema: &'paths Path,
    pub(crate) output: &'paths Path,
    pub(crate) instructions: &'paths Path,
    pub(crate) log: &'paths Path,
    pub(crate) prompt: &'paths str,
}

#[derive(Debug, Deserialize)]
struct ClaudeResult {
    structured_output: Option<Value>,
    result: Option<String>,
}

pub(crate) fn claude_answer(text: &str) -> Result<Option<String>, GateError> {
    let parsed: ClaudeResult = serde_json::from_str(text).map_err(|source| GateError::Json {
        what: "Claude's result".to_owned(),
        source,
    })?;
    match parsed.structured_output {
        Some(value) => serde_json::to_string(&value)
            .map(Some)
            .map_err(|source| GateError::Json {
                what: "Claude's structured output".to_owned(),
                source,
            }),
        None => Ok(parsed
            .result
            .filter(|answer| answer.trim_start().starts_with('{'))),
    }
}

fn expand(
    argument: &str,
    rung: &Rung,
    invocation: &Invocation<'_>,
    schema_text: &str,
) -> Result<String, GateError> {
    let mut values = vec![
        rung.model.as_str(),
        rung.effort.as_str(),
        schema_text,
        invocation.writable_roots,
    ];
    let paths = [
        invocation.worktree,
        invocation.schema,
        invocation.output,
        invocation.instructions,
        invocation.git_common_dir,
        invocation.home,
        invocation.sandbox_settings,
    ];
    for path in paths {
        let text = path.to_str().ok_or_else(|| GateError::Dispatch {
            task: String::new(),
            reason: format!("the path {} is not UTF-8", path.display()),
        })?;
        values.push(text);
    }
    let mut expanded = argument.to_owned();
    for (name, value) in PLACEHOLDERS.iter().zip(values) {
        expanded = expanded.replace(&format!("{{{name}}}"), value);
    }
    Ok(expanded)
}

fn answer(mode: OutputMode, output: &Path, stdout: &Path) -> Result<Option<String>, GateError> {
    match mode {
        OutputMode::File if output.exists() => {
            fs::read_to_string(output)
                .map(Some)
                .map_err(|source| GateError::Read {
                    path: output.to_owned(),
                    source,
                })
        }
        OutputMode::File => Ok(None),
        OutputMode::ClaudeJson => {
            let text = fs::read_to_string(stdout).map_err(|source| GateError::Read {
                path: stdout.to_owned(),
                source,
            })?;
            claude_answer(&text)
        }
    }
}

fn spawn(
    runner: &Runner,
    rung: &Rung,
    invocation: &Invocation<'_>,
    arguments: &[String],
    stdout_path: &Path,
) -> Result<Child, GateError> {
    let stdout = File::create(stdout_path).map_err(|source| GateError::Write {
        path: stdout_path.to_owned(),
        source,
    })?;
    let log = File::create(invocation.log).map_err(|source| GateError::Write {
        path: invocation.log.to_owned(),
        source,
    })?;
    check_command("timeout")
        .args([
            "--kill-after=60",
            &format!("{}m", rung.timeout_minutes),
            &runner.program,
        ])
        .args(arguments)
        .current_dir(invocation.worktree)
        .stdin(Stdio::piped())
        .stdout(stdout)
        .stderr(log)
        .spawn()
        .map_err(|source| GateError::Spawn {
            program: runner.program.clone(),
            source,
        })
}

pub(crate) fn run(
    runner: &Runner,
    rung: &Rung,
    invocation: &Invocation<'_>,
) -> Result<Option<String>, GateError> {
    let raw_schema = fs::read_to_string(invocation.schema).map_err(|source| GateError::Read {
        path: invocation.schema.to_owned(),
        source,
    })?;
    let mut schema: Value =
        serde_json::from_str(&raw_schema).map_err(|source| GateError::Json {
            what: "the answer schema".to_owned(),
            source,
        })?;
    // Claude's validator rejects the 2020-12 $schema marker and resolves the $defs refs fine without it
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
    }
    let schema_text = serde_json::to_string(&schema).map_err(|source| GateError::Json {
        what: "the answer schema".to_owned(),
        source,
    })?;
    let arguments = runner
        .args
        .iter()
        .map(|argument| expand(argument, rung, invocation, &schema_text))
        .collect::<Result<Vec<String>, GateError>>()?;
    let stdout_path = invocation.output.with_extension("stdout");
    let failed = |source| GateError::Spawn {
        program: runner.program.clone(),
        source,
    };
    let mut child = spawn(runner, rung, invocation, &arguments, &stdout_path)?;
    if let Some(mut input) = child.stdin.take() {
        input
            .write_all(invocation.prompt.as_bytes())
            .map_err(failed)?;
    }
    child.wait().map_err(failed)?;
    answer(runner.output, invocation.output, &stdout_path)
}
