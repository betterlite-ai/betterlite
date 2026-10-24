use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::case::Case;
use crate::compare;
use crate::mock::ProviderRequest;

use super::{Captured, HarnessError};

pub(crate) struct ArtifactInput<'a> {
    pub(crate) root: &'a Path,
    pub(crate) case: &'a Case,
    pub(crate) first: &'a Captured,
    pub(crate) second: &'a Captured,
    pub(crate) first_requests: &'a [ProviderRequest],
    pub(crate) second_requests: &'a [ProviderRequest],
    pub(crate) first_log: &'a Path,
    pub(crate) second_log: &'a Path,
    pub(crate) result: Option<&'a compare::Difference>,
}

pub(crate) fn write(input: ArtifactInput<'_>) -> Result<(), HarnessError> {
    let ArtifactInput {
        root,
        case,
        first,
        second,
        first_requests,
        second_requests,
        first_log,
        second_log,
        result,
    } = input;
    let directory = root.join(super::safe_name(&case.name));
    super::create_dir(&directory)?;
    write_json(
        &directory.join("client-litellm.json"),
        &compare::response_document(&first.response),
    )?;
    write_json(
        &directory.join("client-betterlite.json"),
        &compare::response_document(&second.response),
    )?;
    write_json(
        &directory.join("provider-litellm.json"),
        &serde_json::to_value(first_requests).map_err(HarnessError::Json)?,
    )?;
    write_json(
        &directory.join("provider-betterlite.json"),
        &serde_json::to_value(second_requests).map_err(HarnessError::Json)?,
    )?;
    copy_file(first_log, &directory.join("litellm.log"))?;
    copy_file(second_log, &directory.join("betterlite.log"))?;
    let diff = result.map_or(Value::Null, difference_document);
    write_json(&directory.join("diff.json"), &diff)
}

fn difference_document(error: &compare::Difference) -> Value {
    let mut document = serde_json::Map::new();
    document.insert("pointer".to_owned(), Value::String(error.pointer.clone()));
    document.insert("message".to_owned(), Value::String(error.message.clone()));
    Value::Object(document)
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), HarnessError> {
    fs::copy(source, destination).map_err(|error| HarnessError::Filesystem {
        path: destination.to_owned(),
        source: error,
    })?;
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<(), HarnessError> {
    let text = serde_json::to_string_pretty(value).map_err(HarnessError::Json)?;
    super::write_text(path, &text)
}
