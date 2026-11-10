use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingRecord;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::embedding::EmbeddingUsage;
use betterlite_openai::embedding::EmbeddingVector;
use serde_json::Number;

use super::error::VertexTextEmbeddingError;
use super::family::Family;
use super::family::family_of;

pub fn transform_response(
    response: &Json,
    model: &str,
) -> Result<EmbeddingResponse, VertexTextEmbeddingError> {
    let (data, tokens) = match family_of(model) {
        Family::Numeric => tuned_records(response)?,
        Family::Bge => bge_records(response)?,
        Family::Standard => standard_records(response)?,
    };
    Ok(EmbeddingResponse {
        object: String::from("list"),
        data,
        model: model.to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens: tokens,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: tokens,
        },
    })
}

fn standard_records(
    response: &Json,
) -> Result<(Vec<EmbeddingRecord>, u32), VertexTextEmbeddingError> {
    let predictions = predictions(response)?;
    let mut data = Vec::with_capacity(predictions.len());
    let mut tokens = 0_u32;
    for (index, prediction) in predictions.iter().enumerate() {
        let embeddings = field(prediction, "embeddings")?;
        let values = numbers(field(embeddings, "values")?, "values")?;
        let statistics = field(embeddings, "statistics")?;
        let count = token_count(field(statistics, "token_count")?)?;
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding: EmbeddingVector::Floats(values),
            index: u32::try_from(index)
                .map_err(|_source| VertexTextEmbeddingError::IndexOverflow)?,
        });
        tokens = tokens
            .checked_add(count)
            .ok_or(VertexTextEmbeddingError::TokenOverflow)?;
    }
    Ok((data, tokens))
}

fn tuned_records(response: &Json) -> Result<(Vec<EmbeddingRecord>, u32), VertexTextEmbeddingError> {
    let predictions = predictions(response)?;
    let mut data = Vec::with_capacity(predictions.len());
    for (index, prediction) in predictions.iter().enumerate() {
        let Json::List(nested) = prediction else {
            return Err(VertexTextEmbeddingError::PredictionNotList { index });
        };
        let Some(vector) = nested.first() else {
            return Err(VertexTextEmbeddingError::MissingVector { index });
        };
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding: EmbeddingVector::Floats(numbers(vector, "embedding")?),
            index: u32::try_from(index)
                .map_err(|_source| VertexTextEmbeddingError::IndexOverflow)?,
        });
    }
    Ok((data, 0))
}

fn bge_records(response: &Json) -> Result<(Vec<EmbeddingRecord>, u32), VertexTextEmbeddingError> {
    let predictions = predictions(response)?;
    let mut data = Vec::with_capacity(predictions.len());
    for (index, prediction) in predictions.iter().enumerate() {
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding: EmbeddingVector::Floats(numbers(prediction, "embedding")?),
            index: u32::try_from(index)
                .map_err(|_source| VertexTextEmbeddingError::IndexOverflow)?,
        });
    }
    Ok((data, 0))
}

fn predictions(response: &Json) -> Result<&[Json], VertexTextEmbeddingError> {
    let Json::Object(fields) = response else {
        return Err(VertexTextEmbeddingError::MissingPredictions);
    };
    let Some(value) = fields.get("predictions") else {
        return Err(VertexTextEmbeddingError::MissingPredictions);
    };
    let Json::List(items) = value else {
        return Err(VertexTextEmbeddingError::PredictionsNotList);
    };
    Ok(items)
}

fn field<'response>(
    value: &'response Json,
    name: &'static str,
) -> Result<&'response Json, VertexTextEmbeddingError> {
    let Json::Object(fields) = value else {
        return Err(VertexTextEmbeddingError::MissingField { field: name });
    };
    fields
        .get(name)
        .ok_or(VertexTextEmbeddingError::MissingField { field: name })
}

fn numbers(value: &Json, name: &'static str) -> Result<Vec<Number>, VertexTextEmbeddingError> {
    let Json::List(items) = value else {
        return Err(VertexTextEmbeddingError::NotNumbers { field: name });
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Json::Number(number) = item else {
            return Err(VertexTextEmbeddingError::NotNumbers { field: name });
        };
        values.push(number.clone());
    }
    Ok(values)
}

fn token_count(value: &Json) -> Result<u32, VertexTextEmbeddingError> {
    let Json::Number(count) = value else {
        return Err(VertexTextEmbeddingError::NotNumber {
            field: "token_count",
        });
    };
    let Some(whole) = count.as_u64() else {
        return Err(VertexTextEmbeddingError::TokenCount);
    };
    u32::try_from(whole).map_err(|_source| VertexTextEmbeddingError::TokenCount)
}
