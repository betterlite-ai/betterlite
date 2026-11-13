use std::str::from_utf8;

use percent_encoding::percent_decode_str;

use crate::vertex_batch_records::error::BatchRecordsError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchModelPath {
    Publisher { publisher: String, model: String },
    Endpoint { id: String },
}

impl BatchModelPath {
    #[must_use]
    pub fn resource_path(&self) -> String {
        match self {
            Self::Publisher { publisher, model } => {
                format!("publishers/{publisher}/models/{model}")
            }
            Self::Endpoint { id } => format!("endpoints/{id}"),
        }
    }
}

pub fn decode_vertex_batch_identifier(bytes: &[u8]) -> Result<BatchModelPath, BatchRecordsError> {
    let identifier = from_utf8(bytes).map_err(BatchRecordsError::IdentifierUtf8)?;
    parse_identifier(identifier).ok_or_else(|| BatchRecordsError::ModelMissing {
        identifier: identifier.to_owned(),
    })
}

#[must_use]
pub fn is_unmanaged_gcs_input(input_file_id: Option<&str>) -> bool {
    let Some(identifier) = input_file_id else {
        return false;
    };
    identifier.starts_with("gs://") && parse_identifier(identifier).is_some()
}

pub fn batch_job_model(
    identifier: &str,
    project: Option<&str>,
    location: Option<&str>,
) -> Result<String, BatchRecordsError> {
    let model = decode_vertex_batch_identifier(identifier.as_bytes())?;
    let path = model.resource_path();
    if !matches!(model, BatchModelPath::Endpoint { .. }) {
        return Ok(path);
    }
    let Some(project_name) = project.filter(|value| !value.is_empty()) else {
        return Err(BatchRecordsError::ProjectRequired { model: path });
    };
    let location_name = location
        .filter(|value| !value.is_empty())
        .unwrap_or("us-central1");
    Ok(format!(
        "projects/{project_name}/locations/{location_name}/{path}"
    ))
}

pub fn bare_model_name(identifier: &str) -> Result<String, BatchRecordsError> {
    let path = decode_vertex_batch_identifier(identifier.as_bytes())?.resource_path();
    let Some(name) = path.rsplit('/').next() else {
        return Err(BatchRecordsError::ModelMissing {
            identifier: identifier.to_owned(),
        });
    };
    Ok(name.to_owned())
}

fn parse_identifier(identifier: &str) -> Option<BatchModelPath> {
    let decoded = percent_decode_str(identifier).decode_utf8_lossy();
    if let Some((_, model_path)) = decoded.split_once("publishers/") {
        let mut parts = model_path.split('/');
        let publisher = parts.next().unwrap_or_default();
        let marker = parts.next().unwrap_or_default();
        let model = parts.next().unwrap_or_default();
        if marker == "models" && !model.is_empty() {
            return Some(BatchModelPath::Publisher {
                publisher: publisher.to_owned(),
                model: model.to_owned(),
            });
        }
    }
    let (_, endpoint_path) = decoded.rsplit_once("endpoints/")?;
    let endpoint_id = endpoint_path.split('/').next()?;
    if endpoint_id.is_empty() || !endpoint_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(BatchModelPath::Endpoint {
        id: endpoint_id.to_owned(),
    })
}
