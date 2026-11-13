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

pub fn regroup_embeddings(
    rows: &[VertexBatchRow],
    model: Option<&str>,
    next_id: &mut dyn FnMut() -> Uuid,
) -> Result<Vec<OpenAiBatchOutput>, BatchRowsError> {
    let mut groups: IndexMap<String, Vec<(BatchKey, &VertexBatchRow)>> = IndexMap::new();
    for row in rows {
        let key = split_vertex_batch_key(row)?;
        groups
            .entry(key.custom_id.clone())
            .or_default()
            .push((key, row));
    }
    let mut outputs = Vec::with_capacity(groups.len());
    for (custom_id, mut group) in groups {
        group.sort_by_key(|(key, _row)| (key.index, key.total));
        let message = failure_message(&group);
        let body = match message {
            Some(_) => None,
            None => Some(embedding_body(&custom_id, &group, model)?),
        };
        let error = message.map(|text| OpenAiBatchOutputError {
            code: String::from(VERTEX_AI_ERROR),
            message: text,
        });
        outputs.push(openai_batch_output_row(custom_id, body, error, next_id()));
    }
    Ok(outputs)
}

fn failure_message(group: &[(BatchKey, &VertexBatchRow)]) -> Option<String> {
    let failed = group
        .iter()
        .find_map(|(_key, row)| row.status.as_deref().filter(|status| !status.is_empty()));
    if let Some(status) = failed {
        return Some(status.to_owned());
    }
    let positions: Vec<u32> = group.iter().map(|(key, _row)| key.index).collect();
    let total = group
        .iter()
        .map(|(key, _row)| key.total)
        .max()
        .unwrap_or_default();
    positions.iter().copied().ne(0..total).then(|| {
        format!(
            "Vertex returned embeddings for input positions {positions:?} of the {total} requested"
        )
    })
}

fn embedding_body(
    custom_id: &str,
    group: &[(BatchKey, &VertexBatchRow)],
    model: Option<&str>,
) -> Result<IndexMap<String, Json>, BatchRowsError> {
    let shape = |field| BatchRowsError::Shape(custom_id.to_owned(), field);
    let mut data = Vec::with_capacity(group.len());
    let mut tokens = 0_i64;
    for (key, row) in group {
        let response = row.response.object().ok_or_else(|| shape("response"))?;
        let found = match response.get("embedding") {
            Some(Json::Object(fields)) => fields.get("values"),
            _ => None,
        };
        let embedding = found
            .and_then(|values| serde_json::to_value(values).ok())
            .and_then(|encoded| serde_json::from_value(encoded).ok())
            .ok_or_else(|| shape("embedding.values"))?;
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding,
            index: key.index,
        });
        tokens = tokens
            .checked_add(embedding_prompt_token_count(response))
            .ok_or_else(|| shape("usage"))?;
    }
    let prompt_tokens = u32::try_from(tokens).map_err(|_error| shape("usage"))?;
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
    Ok(serde_json::to_value(&body).and_then(serde_json::from_value)?)
}

#[must_use]
pub fn openai_batch_output_row(
    custom_id: String,
    body: Option<IndexMap<String, Json>>,
    error: Option<OpenAiBatchOutputError>,
    id: Uuid,
) -> OpenAiBatchOutput {
    let response = body.map(|fields| OpenAiBatchOutputResponse {
        status_code: 200,
        request_id: fields
            .get("id")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
        body: fields,
    });
    OpenAiBatchOutput {
        id: format!("batch_req_{id}"),
        custom_id,
        response,
        error,
    }
}
