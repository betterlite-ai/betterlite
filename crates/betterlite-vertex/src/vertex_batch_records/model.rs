use std::str::from_utf8;

use percent_encoding::percent_decode_str;

use crate::vertex_batch_records::error::BatchRecordsError;

pub fn decode_vertex_batch_identifier(bytes: &[u8]) -> Result<String, BatchRecordsError> {
    let identifier = from_utf8(bytes).map_err(BatchRecordsError::IdentifierUtf8)?;
    parse_identifier(identifier).ok_or_else(|| BatchRecordsError::ModelMissing {
        identifier: identifier.to_owned(),
    })
}

#[must_use]
pub fn is_unmanaged_gcs_input(input_file_id: Option<&str>) -> bool {
    input_file_id.is_some_and(|identifier| {
        identifier.starts_with("gs://") && parse_identifier(identifier).is_some()
    })
}

pub fn batch_job_model(
    identifier: &str,
    project: Option<&str>,
    location: Option<&str>,
) -> Result<String, BatchRecordsError> {
    let model = decode_vertex_batch_identifier(identifier.as_bytes())?;
    if !model.starts_with("endpoints/") {
        return Ok(model);
    }
    let Some(project_name) = project.filter(|value| !value.is_empty()) else {
        return Err(BatchRecordsError::ProjectRequired { model });
    };
    let location_name = location
        .filter(|value| !value.is_empty())
        .unwrap_or("us-central1");
    Ok(format!(
        "projects/{project_name}/locations/{location_name}/{model}"
    ))
}

pub fn bare_model_name(identifier: &str) -> Result<String, BatchRecordsError> {
    let model = decode_vertex_batch_identifier(identifier.as_bytes())?;
    model.rsplit('/').next().map_or_else(
        || {
            Err(BatchRecordsError::ModelMissing {
                identifier: identifier.to_owned(),
            })
        },
        |name| Ok(name.to_owned()),
    )
}

fn parse_identifier(identifier: &str) -> Option<String> {
    let decoded = percent_decode_str(identifier).decode_utf8_lossy();
    if let Some((_, model_path)) = decoded.split_once("publishers/") {
        let mut parts = model_path.split('/');
        let publisher = parts.next().unwrap_or_default();
        let marker = parts.next().unwrap_or_default();
        let model = parts.next().unwrap_or_default();
        if marker == "models" && !model.is_empty() {
            return Some(format!("publishers/{publisher}/models/{model}"));
        }
    }
    let (_, endpoint_path) = decoded.rsplit_once("endpoints/")?;
    let endpoint_id = endpoint_path.split('/').next()?;
    let digit = !endpoint_id.is_empty() && endpoint_id.bytes().all(|byte| byte.is_ascii_digit());
    digit.then(|| format!("endpoints/{endpoint_id}"))
}
