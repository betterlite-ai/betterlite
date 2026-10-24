use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::case::Case;
use crate::compare;
use crate::mock::ProviderRequest;

use super::HarnessError;
use crate::http::Response;

#[derive(Clone, Copy)]
pub(crate) struct ArtifactInput<'a> {
    pub(crate) root: &'a Path,
    pub(crate) case: &'a Case,
    pub(crate) first: &'a Response,
    pub(crate) second: &'a Response,
    pub(crate) first_requests: &'a [ProviderRequest],
    pub(crate) second_requests: &'a [ProviderRequest],
    pub(crate) first_log: &'a Path,
    pub(crate) second_log: &'a Path,
    pub(crate) result: Option<&'a compare::Difference>,
}

pub(crate) fn write(input: ArtifactInput<'_>) -> Result<PathBuf, HarnessError> {
    let directory = input
        .root
        .join(super::safe_name(&input.case.name))
        .join(super::unique_id()?.to_string());
    super::create_dir(&directory)?;
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

fn copy_file(source: &Path, destination: &Path) -> Result<(), HarnessError> {
    fs::copy(source, destination).map_err(|error| HarnessError::Filesystem {
        path: destination.to_owned(),
        source: error,
    })?;
    Ok(())
}

fn write_json<Data: Serialize + ?Sized>(path: &Path, value: &Data) -> Result<(), HarnessError> {
    let text = serde_json::to_string_pretty(value).map_err(HarnessError::Json)?;
    super::write_text(path, &text)
}
