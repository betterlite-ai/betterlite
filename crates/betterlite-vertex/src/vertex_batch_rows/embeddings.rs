use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingResponse, EmbeddingUsage};
use indexmap::IndexMap;
use uuid::Uuid;

use crate::vertex_batch_records::rows::embedding_prompt_token_count;
use crate::vertex_batch_rows::decode::split_vertex_batch_key;
use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::types::{
    BatchKey, OpenAiBatchOutput, OpenAiBatchOutputError, OpenAiBatchOutputResponse, VertexBatchRow,
};

pub const VERTEX_AI_ERROR: &str = "vertex_ai_error";

type Element<'row> = (BatchKey, Option<&'row str>, &'row IndexMap<String, Json>);

pub fn regroup_embeddings(
    rows: &[VertexBatchRow],
    model: Option<&str>,
    next_id: &mut dyn FnMut() -> Uuid,
) -> Result<Vec<OpenAiBatchOutput>, BatchRowsError> {
    let mut groups: IndexMap<String, Vec<Element<'_>>> = IndexMap::new();
    for row in rows {
        let key = split_vertex_batch_key(row)?;
        let Some(response) = row.response_object() else {
            return Err(BatchRowsError::Shape {
                custom_id: key.custom_id,
                field: "response",
            });
        };
        groups.entry(key.custom_id.clone()).or_default().push((
            key,
            row.status.as_deref(),
            response,
        ));
    }
    let mut outputs = Vec::with_capacity(groups.len());
    for (custom_id, mut group) in groups {
        group.sort_by_key(|(key, _status, _response)| (key.index, key.total));
        outputs.push(embedding_output(&custom_id, &group, model, next_id())?);
    }
    Ok(outputs)
}

fn embedding_output(
    custom_id: &str,
    group: &[Element<'_>],
    model: Option<&str>,
    id: Uuid,
) -> Result<OpenAiBatchOutput, BatchRowsError> {
    let failure = group
        .iter()
        .find_map(|(_key, status, _response)| status.filter(|text| !text.is_empty()));
    let mut total = 0_u32;
    let mut positions = Vec::with_capacity(group.len());
    for (key, _status, _response) in group {
        total = total.max(key.total);
        positions.push(key.index);
    }
    let error = failure.map_or_else(
        || {
            positions.iter().copied().ne(0..total).then(|| {
                output_error(&format!(
                    "Vertex returned embeddings for input positions {positions:?} of the {total} requested"
                ))
            })
        },
        |status| Some(output_error(status)),
    );
    let body = match error {
        Some(_) => None,
        None => Some(embedding_body(custom_id, group, model)?),
    };
    Ok(openai_batch_output_row(
        custom_id.to_owned(),
        body,
        error,
        id,
    ))
}

fn embedding_body(
    custom_id: &str,
    group: &[Element<'_>],
    model: Option<&str>,
) -> Result<Json, BatchRowsError> {
    let mut data = Vec::with_capacity(group.len());
    let mut tokens: i64 = 0;
    for (offset, (_key, _status, response)) in group.iter().enumerate() {
        let field = match response.get("embedding") {
            Some(Json::Object(fields)) => fields.get("values"),
            _ => None,
        };
        let Some(values) = field else {
            return Err(shape(custom_id, "embedding.values"));
        };
        let encoded =
            serde_json::to_value(values).map_err(|source| BatchRowsError::Decode { source })?;
        let embedding = serde_json::from_value(encoded)
            .map_err(|_error| shape(custom_id, "embedding.values"))?;
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding,
            index: u32::try_from(offset).map_err(|_error| shape(custom_id, "index"))?,
        });
        tokens = tokens
            .checked_add(embedding_prompt_token_count(response))
            .ok_or_else(|| shape(custom_id, "usage"))?;
    }
    let prompt_tokens = u32::try_from(tokens).map_err(|_error| shape(custom_id, "usage"))?;
    let body = EmbeddingResponse {
        object: String::from("list"),
        data,
        model: model.unwrap_or_default().to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: prompt_tokens,
        },
    };
    let encoded =
        serde_json::to_value(&body).map_err(|source| BatchRowsError::Decode { source })?;
    serde_json::from_value(encoded).map_err(|source| BatchRowsError::Decode { source })
}

#[must_use]
pub fn openai_batch_output_row(
    custom_id: String,
    body: Option<Json>,
    error: Option<OpenAiBatchOutputError>,
    id: Uuid,
) -> OpenAiBatchOutput {
    let response = body.map(|encoded| OpenAiBatchOutputResponse {
        status_code: 200,
        request_id: request_id(&encoded),
        body: encoded,
    });
    OpenAiBatchOutput {
        id: format!("batch_req_{id}"),
        custom_id,
        response,
        error,
    }
}

fn request_id(body: &Json) -> Json {
    let Json::Object(fields) = body else {
        return Json::Text(String::new());
    };
    fields
        .get("id")
        .cloned()
        .unwrap_or_else(|| Json::Text(String::new()))
}

fn output_error(message: &str) -> OpenAiBatchOutputError {
    OpenAiBatchOutputError {
        code: String::from(VERTEX_AI_ERROR),
        message: message.to_owned(),
    }
}

fn shape(custom_id: &str, field: &'static str) -> BatchRowsError {
    BatchRowsError::Shape {
        custom_id: custom_id.to_owned(),
        field,
    }
}
