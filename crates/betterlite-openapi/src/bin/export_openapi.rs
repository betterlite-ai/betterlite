use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::process::ExitCode;

use betterlite_openapi::document_builder::{DocumentInputs, build_document};
use betterlite_openapi::snapshot_render::{SnapshotDrift, check_snapshot, render_snapshot};
use thiserror::Error;

#[derive(Debug, Error)]
enum ExportError {
    #[error(
        "usage: export_openapi --write <inputs.json> <output.json> | --check <inputs.json> <output.json>"
    )]
    Usage,
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("cannot parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("the snapshot file {path} is missing\nrun: {command}")]
    MissingSnapshot { path: String, command: String },
    #[error("{message}\nrun: {command}")]
    Snapshot {
        message: String,
        command: String,
        #[source]
        drift: SnapshotDrift,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _written = writeln!(io::stderr().lock(), "export_openapi: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), ExportError> {
    let mut arguments = Vec::new();
    for argument in env::args_os() {
        match argument.into_string() {
            Ok(text) => arguments.push(text),
            Err(_) => return Err(ExportError::Usage),
        }
    }
    let [_, mode, inputs, output] = arguments.as_slice() else {
        return Err(ExportError::Usage);
    };
    let inputs_text = fs::read_to_string(inputs).map_err(|source| ExportError::Read {
        path: inputs.clone(),
        source,
    })?;
    let document: DocumentInputs =
        serde_json::from_str(&inputs_text).map_err(|source| ExportError::Parse {
            path: inputs.clone(),
            source,
        })?;
    let rendered = render_snapshot(&build_document(&document));
    match mode.as_str() {
        "--write" => fs::write(output, rendered).map_err(|source| ExportError::Write {
            path: output.clone(),
            source,
        }),
        "--check" => check_snapshot_file(output, inputs, &rendered),
        _ => Err(ExportError::Usage),
    }
}

fn check_snapshot_file(output: &str, inputs: &str, rendered: &str) -> Result<(), ExportError> {
    let command = format!(
        "cargo run --locked -p betterlite-openapi --bin export_openapi -- --write {inputs} {output}"
    );
    let committed = match fs::read_to_string(output) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ExportError::MissingSnapshot {
                path: output.to_owned(),
                command,
            });
        }
        Err(source) => {
            return Err(ExportError::Read {
                path: output.to_owned(),
                source,
            });
        }
    };
    match check_snapshot(&committed, rendered) {
        Ok(()) => Ok(()),
        Err(drift) => {
            let message = drift.message();
            Err(ExportError::Snapshot {
                message,
                command,
                drift,
            })
        }
    }
}
