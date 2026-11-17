use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::vertex_batch_download::records::ConversionContext;
use crate::vertex_batch_download::records::body::completion_body;
use crate::vertex_batch_download::records::error::RecordsError;
use crate::vertex_batch_rows::correlation::{CorrelationInput, decode_batch_correlation};
use crate::vertex_batch_rows::decode::decode_vertex_batch_row;
use crate::vertex_batch_rows::embeddings::{VERTEX_AI_ERROR, openai_batch_output_row};
use crate::vertex_batch_rows::labels::{UNKNOWN_ID, custom_id_from_labels};
use crate::vertex_batch_rows::types::{
    OpenAiBatchOutput, OpenAiBatchOutputError, RowField, VertexBatchRow,
};

const DEFAULT_MODEL: &str = "gemini-1.5-flash-001";
const TRANSFORMATION_ERROR: &str = "transformation_error";

pub(crate) fn transform_line(
    line: &[u8],
    context: &mut ConversionContext,
) -> Result<Vec<u8>, RecordsError> {
    let row = decode_vertex_batch_row(line)?;
    let output = transform_row(&row, context)?;
    serde_json::to_vec(&output).map_err(RecordsError::Encode)
}

// a status is vertex_ai_error, a failed conversion a transformation_error, and a broken row fails the caller
fn transform_row(
    row: &VertexBatchRow,
    context: &mut ConversionContext,
) -> Result<OpenAiBatchOutput, RecordsError> {
    let custom_id = row_custom_id(row)?;
    if let Some(status) = row.status.as_deref().filter(|status| !status.is_empty()) {
        let error = OpenAiBatchOutputError {
            code: String::from(VERTEX_AI_ERROR),
            message: status.to_owned(),
        };
        return Ok(openai_batch_output_row(
            custom_id,
            None,
            Some(error),
            (context.ids)(),
        ));
    }
    let empty = IndexMap::new();
    let response = match &row.response {
        RowField::Absent => &empty,
        RowField::Value(Json::Object(fields)) => fields,
        RowField::Value(_) => return Err(RecordsError::Response),
    };
    // a present modelVersion overrides this inside the shared conversion, and an "@" suffix is its to strip
    let model = match response.get("modelVersion") {
        None => String::from(DEFAULT_MODEL),
        Some(Json::Text(version)) => version.clone(),
        Some(_) => return Err(RecordsError::ModelVersion),
    };
    let body = match completion_body(response, &model, context) {
        Ok(fields) => fields,
        Err(error) => {
            let message = format!("Failed to transform response: {error}");
            return Ok(error_row(custom_id, &message, (context.ids)()));
        }
    };
    Ok(openai_batch_output_row(
        custom_id,
        Some(body),
        None,
        (context.ids)(),
    ))
}

fn row_custom_id(row: &VertexBatchRow) -> Result<String, RecordsError> {
    if let Some(key) = row.key.as_deref() {
        return Ok(decode_batch_correlation(CorrelationInput::Key(
            key.as_bytes(),
        ))?);
    }
    let labels = row.request.object().and_then(|fields| fields.get("labels"));
    match labels {
        None | Some(Json::Null) => Ok(String::from(UNKNOWN_ID)),
        Some(Json::Object(fields)) => Ok(custom_id_from_labels(fields)?),
        Some(_) => Err(RecordsError::Labels),
    }
}

fn error_row(custom_id: String, message: &str, id: Uuid) -> OpenAiBatchOutput {
    let error = OpenAiBatchOutputError {
        code: String::from(TRANSFORMATION_ERROR),
        message: message.to_owned(),
    };
    openai_batch_output_row(custom_id, None, Some(error), id)
}
