use std::str::from_utf8;

use betterlite_context::cloud_storage::{
    VERTEX_AI_MANAGED_GCS_PREFIX, build_object_name, sanitize_path,
};
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media_metadata::{FileBody, FileData, extract_file_metadata};
use uuid::Uuid;

use crate::model_urls::model_name::get_fine_tuned_endpoint_id;
use crate::vertex_batch_upload::line::{BatchLine, decode_openai_batch_line};
use crate::vertex_gcs_access::naming::VERTEX_UPLOADS_PREFIX;

use super::FileRequest;
use super::error::VertexFilesError;

const PASSTHROUGH_PREFIX: &str = "litellm-vertex-files/passthrough/";
const PUBLISHERS_MODELS: &str = "publishers/google/models";

// a numeric endpoint id names a fine-tuned Gemini deployment, which lives under endpoints/<id>
fn managed_object_name(model: &str, passthrough: bool, id: Uuid) -> String {
    let model_path = match get_fine_tuned_endpoint_id(model) {
        Some(endpoint) => format!("endpoints/{endpoint}"),
        None if model.contains(PUBLISHERS_MODELS) => model.to_owned(),
        None => format!("{PUBLISHERS_MODELS}/{model}"),
    };
    let safe_path = sanitize_path(Some(model_path.as_str()), "model");
    let prefix = if passthrough {
        PASSTHROUGH_PREFIX
    } else {
        VERTEX_AI_MANAGED_GCS_PREFIX
    };
    format!("{prefix}{safe_path}/{}", id.hyphenated())
}

fn batch_line_model(first: &BatchLine) -> Result<String, VertexFilesError> {
    let body = match first.body.as_ref() {
        None | Some(Json::Null) => return Ok(String::new()),
        Some(body) => body,
    };
    let Some(object) = body.as_object() else {
        return Err(VertexFilesError::BatchBody);
    };
    match object.get("model") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(model)) => Ok(model.clone()),
        Some(_) => Err(VertexFilesError::BatchModel),
    }
}

fn content_text(file: &FileData) -> Result<&str, VertexFilesError> {
    let body = match file {
        FileData::Direct(body) => body,
        FileData::Tuple2 { content, .. }
        | FileData::Tuple3 { content, .. }
        | FileData::Tuple4 { content, .. } => content,
        FileData::InMemoryFile { content, .. } => {
            return from_utf8(content).map_err(VertexFilesError::ContentUtf8);
        }
    };
    match body {
        FileBody::BareString(text) => Ok(text),
        FileBody::Bytes(bytes) => from_utf8(bytes).map_err(VertexFilesError::ContentUtf8),
    }
}

// only the first non-blank line is read, so a large upload is never materialized to name its object
fn first_entry(file: &FileData) -> Result<Option<BatchLine>, VertexFilesError> {
    for line in content_text(file)?.split('\n') {
        if let Some(entry) = decode_openai_batch_line(line.as_bytes())? {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

pub fn get_object_name(request: &FileRequest<'_>, id: Uuid) -> Result<String, VertexFilesError> {
    let deployment_model = request
        .deployment_model
        .filter(|model| !model.is_empty())
        .map(|model| model.strip_prefix("vertex_ai/").unwrap_or(model));
    // VertexAIError(status_code=400): a passthrough upload carries no model row to name the object from
    if request.purpose == CreatePurpose::Batch && request.passthrough {
        let model = deployment_model.ok_or(VertexFilesError::PassthroughModel)?;
        return Ok(managed_object_name(model, true, id));
    }
    // the deployment's own model wins, so a JSONL body.model can only name a direct SDK upload
    if request.purpose == CreatePurpose::Batch
        && let Some(first) = first_entry(request.file)?
    {
        let model = deployment_model
            .map_or_else(|| batch_line_model(&first), |model| Ok(model.to_owned()))?;
        return Ok(managed_object_name(&model, false, id));
    }
    let (filename, _content_type) = extract_file_metadata(request.file);
    Ok(build_object_name(
        VERTEX_UPLOADS_PREFIX,
        filename.as_deref(),
        "file",
        id,
    ))
}

mod tests;
