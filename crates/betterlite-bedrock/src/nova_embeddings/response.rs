use crate::image_models::params::{Field, Params};

use super::error::NovaEmbeddingsError;

#[derive(Debug, Clone, PartialEq)]
pub struct NovaEmbeddingRecord {
    pub embedding: Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovaPromptTokensDetails {
    pub image_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovaUsage {
    pub prompt_tokens: u64,
    pub total_tokens: u64,
    pub prompt_tokens_details: Option<NovaPromptTokensDetails>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NovaEmbeddingResponse {
    pub data: Vec<NovaEmbeddingRecord>,
    pub model: String,
    pub usage: NovaUsage,
    pub invocation_arn: Option<Field>,
}

pub fn transform_response(
    response_list: &[Params],
    model: &str,
    batch_data: Option<&[Params]>,
) -> Result<NovaEmbeddingResponse, NovaEmbeddingsError> {
    let (data, total_tokens) = collect_embeddings(response_list)?;
    let image_count = batch_data.map_or(0, count_images);
    let prompt_tokens_details =
        (image_count > 0).then_some(NovaPromptTokensDetails { image_count });

    Ok(NovaEmbeddingResponse {
        data,
        model: model.to_owned(),
        usage: NovaUsage {
            prompt_tokens: total_tokens,
            total_tokens,
            prompt_tokens_details,
        },
        invocation_arn: None,
    })
}

#[must_use]
pub fn transform_invoke_response(response: &Params, model: &str) -> NovaEmbeddingResponse {
    let invocation_arn = response
        .get("invocationArn")
        .cloned()
        .unwrap_or_else(|| Field::Text(String::new()));
    NovaEmbeddingResponse {
        data: vec![NovaEmbeddingRecord {
            embedding: Field::List(Vec::new()),
        }],
        model: model.to_owned(),
        usage: NovaUsage {
            prompt_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
        },
        invocation_arn: Some(invocation_arn),
    }
}

fn collect_embeddings(
    response_list: &[Params],
) -> Result<(Vec<NovaEmbeddingRecord>, u64), NovaEmbeddingsError> {
    let mut data = Vec::new();
    let mut total_tokens: u64 = 0;
    for response in response_list {
        for (embedding, estimate) in retained_vectors(response)? {
            total_tokens = total_tokens
                .checked_add(estimate)
                .ok_or(NovaEmbeddingsError::TokenOverflow)?;
            data.push(NovaEmbeddingRecord { embedding });
        }
    }
    Ok((data, total_tokens))
}

// An "embeddings" list, even an empty one, always wins over a direct "embedding" fallback
fn retained_vectors(response: &Params) -> Result<Vec<(Field, u64)>, NovaEmbeddingsError> {
    if let Some(Field::List(items)) = response.get("embeddings") {
        return items.iter().filter_map(embeddings_list_entry).collect();
    }
    match response.get("embedding") {
        Some(embedding_field) => {
            let estimate = embedding_length(embedding_field)?;
            Ok(vec![(embedding_field.clone(), estimate)])
        }
        None => Ok(Vec::new()),
    }
}

fn embeddings_list_entry(item: &Field) -> Option<Result<(Field, u64), NovaEmbeddingsError>> {
    let Field::Mapping(mapping) = item else {
        return None;
    };
    let embedding_field = mapping.get("embedding")?;
    Some(embedding_list_item(mapping, embedding_field))
}

fn embedding_list_item(
    item: &Params,
    embedding_field: &Field,
) -> Result<(Field, u64), NovaEmbeddingsError> {
    let estimate = match item.get("truncatedCharLength") {
        Some(field) => as_u64(field, "truncatedCharLength")?
            .checked_div(4)
            .unwrap_or(0),
        None => embedding_length(embedding_field)?,
    };
    Ok((embedding_field.clone(), estimate))
}

// embedding is list | str; anything else has no __len__ in Python and errors.
fn embedding_length(field: &Field) -> Result<u64, NovaEmbeddingsError> {
    let length = match field {
        Field::List(items) => items.len(),
        Field::Text(text) => text.chars().count(),
        Field::Null | Field::Bool(_) | Field::Int(_) | Field::Float(_) | Field::Mapping(_) => {
            return Err(NovaEmbeddingsError::MalformedField { field: "embedding" });
        }
    };
    let length_u64 = u64::try_from(length).map_err(|_source| NovaEmbeddingsError::TokenOverflow)?;
    Ok(length_u64.checked_div(4).unwrap_or(0))
}

fn as_u64(field: &Field, name: &'static str) -> Result<u64, NovaEmbeddingsError> {
    match field {
        Field::Int(value) => u64::try_from(*value)
            .map_err(|_source| NovaEmbeddingsError::MalformedField { field: name }),
        Field::Null
        | Field::Bool(_)
        | Field::Float(_)
        | Field::Text(_)
        | Field::List(_)
        | Field::Mapping(_) => Err(NovaEmbeddingsError::MalformedField { field: name }),
    }
}

fn count_images(batch_data: &[Params]) -> u32 {
    let mut count = 0_u32;
    for request_data in batch_data {
        let params = request_data
            .get("singleEmbeddingParams")
            .or_else(|| request_data.get("segmentedEmbeddingParams"));
        if let Some(Field::Mapping(map)) = params
            && map.contains_key("image")
        {
            count = count.saturating_add(1);
        }
    }
    count
}
