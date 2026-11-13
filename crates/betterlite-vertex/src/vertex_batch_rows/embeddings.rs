use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use indexmap::IndexMap;
use uuid::Uuid;

use crate::vertex_batch_records::rows::embedding_prompt_token_count;
use crate::vertex_batch_rows::decode::split_vertex_batch_key;
use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::types::{
    BatchKey, OpenAiBatchOutput, OpenAiBatchOutputError, OpenAiBatchOutputResponse, VertexBatchRow,
};

pub const VERTEX_AI_ERROR: &str = "vertex_ai_error";

struct Element<'row> {
    key: BatchKey,
    status: Option<&'row str>,
    response: &'row IndexMap<String, Json>,
}

pub fn regroup_embeddings<F>(
    rows: &[VertexBatchRow],
    model: Option<&str>,
    mut next_id: F,
) -> Result<Vec<OpenAiBatchOutput>, BatchRowsError>
where
    F: FnMut() -> Uuid,
{
    let mut elements = Vec::with_capacity(rows.len());
    for row in rows {
        let key = split_vertex_batch_key(row)?;
        let response = row
            .response_object()
            .ok_or(BatchRowsError::Shape { field: "response" })?;
        elements.push(Element {
            key,
            status: row.status.as_deref(),
            response,
        });
    }
    let mut groups: IndexMap<String, Vec<&Element<'_>>> = IndexMap::new();
    for element in &elements {
        groups
            .entry(element.key.custom_id.clone())
            .or_default()
            .push(element);
    }
    let mut outputs = Vec::with_capacity(groups.len());
    for (custom_id, mut group) in groups {
        group.sort_by_key(|element| (element.key.index, element.key.total));
        outputs.push(embedding_output(&custom_id, &group, model, next_id())?);
    }
    Ok(outputs)
}

pub fn openai_batch_output_row(
    custom_id: String,
    body: Option<Json>,
    error: Option<OpenAiBatchOutputError>,
    id: Uuid,
) -> Result<OpenAiBatchOutput, BatchRowsError> {
    let response = match body {
        Some(encoded) => Some(OpenAiBatchOutputResponse {
            status_code: 200,
            request_id: request_id(&encoded)?,
            body: encoded,
        }),
        None => None,
    };
    Ok(OpenAiBatchOutput {
        id: format!("batch_req_{id}"),
        custom_id,
        response,
        error,
    })
}

fn embedding_output(
    custom_id: &str,
    group: &[&Element<'_>],
    model: Option<&str>,
    id: Uuid,
) -> Result<OpenAiBatchOutput, BatchRowsError> {
    let failure = group
        .iter()
        .find_map(|element| element.status.filter(|status| !status.is_empty()));
    if let Some(status) = failure {
        return openai_batch_output_row(custom_id.to_owned(), None, Some(error(status)), id);
    }
    let total = group
        .iter()
        .fold(0_u32, |largest, element| largest.max(element.key.total));
    let positions: Vec<u32> = group.iter().map(|element| element.key.index).collect();
    if !positions.iter().copied().eq(0..total) {
        let message = format!(
            "Vertex returned embeddings for input positions {} of the {total} requested",
            bracketed_list(&positions)
        );
        return openai_batch_output_row(custom_id.to_owned(), None, Some(error(&message)), id);
    }
    let body = embedding_body(custom_id, group, model)?;
    openai_batch_output_row(custom_id.to_owned(), Some(body), None, id)
}

fn embedding_body(
    custom_id: &str,
    group: &[&Element<'_>],
    model: Option<&str>,
) -> Result<Json, BatchRowsError> {
    let mut data = Vec::with_capacity(group.len());
    let mut tokens: i64 = 0;
    for (offset, element) in group.iter().enumerate() {
        let index = u32::try_from(offset).map_err(|_error| BatchRowsError::Tokens {
            custom_id: custom_id.to_owned(),
        })?;
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding: embedding_values(element.response, custom_id)?,
            index,
        });
        tokens = tokens
            .checked_add(embedding_prompt_token_count(element.response))
            .ok_or_else(|| BatchRowsError::Tokens {
                custom_id: custom_id.to_owned(),
            })?;
    }
    let prompt_tokens = u32::try_from(tokens).map_err(|_error| BatchRowsError::Tokens {
        custom_id: custom_id.to_owned(),
    })?;
    let response = EmbeddingResponse {
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
    let encoded = serde_json::to_value(&response).map_err(|source| BatchRowsError::Encode {
        custom_id: custom_id.to_owned(),
        source,
    })?;
    serde_json::from_value(encoded).map_err(|source| BatchRowsError::Encode {
        custom_id: custom_id.to_owned(),
        source,
    })
}

fn embedding_values(
    response: &IndexMap<String, Json>,
    custom_id: &str,
) -> Result<EmbeddingVector, BatchRowsError> {
    let field = match response.get("embedding") {
        Some(Json::Object(embedding)) => embedding.get("values"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    let Some(Json::List(values)) = field else {
        return Err(BatchRowsError::Values {
            custom_id: custom_id.to_owned(),
        });
    };
    let mut floats = Vec::with_capacity(values.len());
    for value in values {
        let Json::Number(number) = value else {
            return Err(BatchRowsError::Values {
                custom_id: custom_id.to_owned(),
            });
        };
        floats.push(number.clone());
    }
    Ok(EmbeddingVector::Floats(floats))
}

fn request_id(body: &Json) -> Result<String, BatchRowsError> {
    match body {
        Json::Object(fields) => match fields.get("id") {
            Some(Json::Text(id)) => Ok(id.clone()),
            Some(Json::Null) | None => Ok(String::new()),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                Err(BatchRowsError::Shape {
                    field: "response.body.id",
                })
            }
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(BatchRowsError::Shape {
                field: "response.body",
            })
        }
    }
}

fn error(message: &str) -> OpenAiBatchOutputError {
    OpenAiBatchOutputError {
        code: String::from(VERTEX_AI_ERROR),
        message: message.to_owned(),
    }
}

fn bracketed_list(positions: &[u32]) -> String {
    let joined = positions
        .iter()
        .map(u32::to_string)
        .collect::<Vec<String>>()
        .join(", ");
    format!("[{joined}]")
}
