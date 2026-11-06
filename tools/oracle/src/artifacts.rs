use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::case::Case;
use crate::compare;
use crate::mock::ProviderRequest;
use crate::paths::{safe_name, unique_id};
use crate::sdk_http::capture::Capture;

use crate::OracleError;
use crate::http::Response;

#[derive(Clone, Copy)]
pub(crate) struct ArtifactInput<'input> {
    pub(crate) root: &'input Path,
    pub(crate) case: &'input Case,
    pub(crate) first: &'input Response,
    pub(crate) second: &'input Response,
    pub(crate) first_requests: &'input [ProviderRequest],
    pub(crate) second_requests: &'input [ProviderRequest],
    pub(crate) first_log: &'input Path,
    pub(crate) second_log: &'input Path,
    pub(crate) result: Option<&'input compare::Difference>,
}

pub(crate) fn write(input: ArtifactInput<'_>) -> Result<PathBuf, OracleError> {
    let directory = fresh_directory(input.root, &input.case.name)?;
    for (name, response, requests, log) in [
        (
            "litellm",
            input.first,
            input.first_requests,
            input.first_log,
        ),
        (
            "betterlite",
            input.second,
            input.second_requests,
            input.second_log,
        ),
    ] {
        write_json(&directory.join(format!("client-{name}.json")), response)?;
        write_json(&directory.join(format!("provider-{name}.json")), requests)?;
        copy_file(log, &directory.join(format!("{name}.log")))?;
    }
    write_json(&directory.join("diff.json"), &input.result)?;
    Ok(directory)
}

pub(crate) struct SdkArtifactInput<'input> {
    pub(crate) root: &'input Path,
    pub(crate) name: &'input str,
    pub(crate) labels: [&'input str; 2],
    pub(crate) captures: [&'input Capture; 2],
    pub(crate) difference: &'input compare::Difference,
}

// both logs and captures stay for a mismatch and for a crash alike
pub(crate) fn write_sdk(input: &SdkArtifactInput<'_>) -> Result<PathBuf, OracleError> {
    let directory = fresh_directory(&input.root.join("sdk-http"), input.name)?;
    for (label, capture) in input.labels.iter().zip(input.captures) {
        write_json(
            &directory.join(format!("client-{label}.json")),
            &client_value(capture)?,
        )?;
        write_json(
            &directory.join(format!("provider-{label}.json")),
            &capture.requests,
        )?;
        let log = directory.join(format!("{label}.log"));
        fs::write(&log, &capture.execution.log)
            .map_err(|source| OracleError::Filesystem { path: log, source })?;
    }
    write_json(&directory.join("diff.json"), input.difference)?;
    Ok(directory)
}

fn client_value(capture: &Capture) -> Result<Value, OracleError> {
    match &capture.execution.result {
        Ok(result) => serde_json::to_value(result).map_err(OracleError::Json),
        Err(error) => {
            let mut failed = serde_json::Map::new();
            failed.insert("error".to_owned(), Value::String(error.to_string()));
            Ok(Value::Object(failed))
        }
    }
}

fn fresh_directory(root: &Path, case_name: &str) -> Result<PathBuf, OracleError> {
    let directory = root.join(safe_name(case_name)).join(unique_id()?);
    fs::create_dir_all(&directory).map_err(|source| OracleError::Filesystem {
        path: directory.clone(),
        source,
    })?;
    Ok(directory)
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), OracleError> {
    fs::copy(source, destination).map_err(|error| OracleError::Filesystem {
        path: destination.to_owned(),
        source: error,
    })?;
    Ok(())
}

fn write_json<Data>(path: &Path, value: &Data) -> Result<(), OracleError>
where
    Data: Serialize + ?Sized,
{
    let text = serde_json::to_string_pretty(value).map_err(OracleError::Json)?;
    fs::write(path, text).map_err(|source| OracleError::Filesystem {
        path: path.to_owned(),
        source,
    })
}

pub(crate) fn prepare(
    root: &Path,
    case: &Case,
    upstream: &str,
) -> Result<(PathBuf, PathBuf), OracleError> {
    let directory = root.join(format!("group-{}", unique_id()?));
    fs::create_dir_all(&directory).map_err(|source| OracleError::Filesystem {
        path: directory.clone(),
        source,
    })?;
    let path = directory.join("config.yaml");
    let config = case.config.replace("{{upstream}}", upstream);
    fs::write(&path, config).map_err(|source| OracleError::Filesystem {
        path: path.clone(),
        source,
    })?;
    Ok((directory, path))
}
