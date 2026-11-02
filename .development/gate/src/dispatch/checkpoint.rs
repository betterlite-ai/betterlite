mod tests;

use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, ErrorKind, Read as _};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::progress::{Progress, WorkerSession};
use crate::error::GateError;
use crate::spec::agents::{Rung, Runner};

#[derive(Serialize, Deserialize)]
struct Started {
    runner: String,
    model: String,
    session: Option<String>,
}

pub(super) fn begin(
    output: &Path,
    runner: &Runner,
    rung: &Rung,
    session: Option<&str>,
) -> Result<(), GateError> {
    let started = Started {
        runner: runner.id.clone(),
        model: rung.model.clone(),
        session: session.map(str::to_owned),
    };
    let path = output.with_extension("started.json");
    let text = serde_json::to_string(&started).map_err(|source| GateError::Json {
        what: "the invocation checkpoint".to_owned(),
        source,
    })?;
    fs::write(&path, text).map_err(|source| GateError::Write { path, source })
}

pub(super) fn session(output: &Path) -> Result<Option<String>, GateError> {
    let path = output.with_extension("stdout");
    let opened = match File::open(&path) {
        Ok(file) => Some(file),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(source) => return Err(GateError::Read { path, source }),
    };
    for result in opened
        .into_iter()
        .flat_map(|file| BufReader::new(file.take(8 * 1024 * 1024)).lines())
    {
        let line = result.map_err(|source| GateError::Read {
            path: path.clone(),
            source,
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = if value.get("type").and_then(Value::as_str) == Some("thread.started") {
            value.get("thread_id")
        } else if value.get("type").and_then(Value::as_str) == Some("result") {
            value.get("session_id")
        } else {
            None
        };
        if let Some(text) = id.and_then(Value::as_str).filter(|text| !text.is_empty()) {
            return Ok(Some(text.to_owned()));
        }
    }
    Ok(started(output)?.and_then(|started| started.session))
}

pub(super) fn finish(
    output: &Path,
    elapsed: Duration,
    exit_code: Option<i32>,
) -> Result<(), GateError> {
    let value = serde_json::json!({"elapsed_ms": elapsed.as_millis(), "exit_code": exit_code});
    let path = output.with_extension("finished.json");
    fs::write(&path, value.to_string()).map_err(|source| GateError::Write { path, source })
}

fn started(output: &Path) -> Result<Option<Started>, GateError> {
    let path = output.with_extension("started.json");
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| GateError::Json {
                what: path.display().to_string(),
                source,
            }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(GateError::Read { path, source }),
    }
}

pub(super) fn recover(output: &Path) -> Result<Option<WorkerSession>, GateError> {
    if output.with_extension("handled").exists() {
        return Ok(None);
    }
    let Some(started) = started(output)? else {
        return Ok(None);
    };
    Ok(session(output)?.map(|id| WorkerSession {
        runner: started.runner,
        model: started.model,
        id,
    }))
}

pub(super) fn save_turn(
    journal: &Path,
    progress: &Progress,
    interrupted: bool,
) -> Result<(), GateError> {
    progress.save(journal)?;
    if interrupted {
        return Ok(());
    }
    let path = journal.join(format!("attempt-{}-worker.handled", progress.number));
    fs::write(&path, "handled\n").map_err(|source| GateError::Write { path, source })
}

pub(super) fn restore(
    journal: &Path,
    progress: &mut Progress,
    runner: &Runner,
    rung: &Rung,
) -> Result<bool, GateError> {
    let previous = journal.join(format!("attempt-{}-worker.json", progress.number));
    if let Some(saved) = recover(&previous)?
        && saved.runner == runner.id
        && saved.model == rung.model
    {
        progress.session = Some(saved);
        return Ok(true);
    }
    Ok(false)
}
