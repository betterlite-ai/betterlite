use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::case::Case;
use crate::compare;
use crate::mock::ProviderRequest;

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
    let directory = input
        .root
        .join(super::safe_name(&input.case.name))
        .join(super::unique_id()?);
    fs::create_dir_all(&directory).map_err(|source| OracleError::Filesystem {
        path: directory.clone(),
        source,
    })?;
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
    let directory = root.join(format!("group-{}", super::unique_id()?));
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
