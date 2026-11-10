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
    let (data, tokens) = records(response, family_of(model))?;
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

fn records(
    response: &Json,
    family: Family,
) -> Result<(Vec<EmbeddingRecord>, u32), VertexTextEmbeddingError> {
    let predictions = predictions(response)?;
    let mut data = Vec::with_capacity(predictions.len());
    let mut tokens = 0_u32;
    for (index, prediction) in predictions.iter().enumerate() {
        let (vector, name) = match family {
            Family::Numeric => (nested_vector(prediction, index)?, "embedding"),
            Family::Bge => (prediction, "embedding"),
            Family::Standard => {
                let embeddings = field(prediction, "embeddings")?;
                let values = field(embeddings, "values")?;
                let statistics = field(embeddings, "statistics")?;
                let count = token_count(field(statistics, "token_count")?)?;
                tokens = tokens
                    .checked_add(count)
                    .ok_or(VertexTextEmbeddingError::TokenOverflow)?;
                (values, "values")
            }
        };
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding: EmbeddingVector::Floats(numbers(vector, name)?),
            index: u32::try_from(index)
                .map_err(|_source| VertexTextEmbeddingError::IndexOverflow)?,
        });
    }
    Ok((data, tokens))
}

fn nested_vector(prediction: &Json, index: usize) -> Result<&Json, VertexTextEmbeddingError> {
    let Json::List(nested) = prediction else {
        return Err(VertexTextEmbeddingError::PredictionNotList { index });
    };
    nested
        .first()
        .ok_or(VertexTextEmbeddingError::MissingVector { index })
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
