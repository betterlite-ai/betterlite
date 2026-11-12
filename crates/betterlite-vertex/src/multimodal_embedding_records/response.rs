use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingRecord;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::embedding::EmbeddingUsage;
use betterlite_openai::embedding::EmbeddingVector;

use super::error::MultimodalEmbeddingError;
use super::usage::MediaPromptTokensDetails;
use super::usage::calculate_usage;

pub const RESPONSE_BODY_LIMIT: usize = 0x0200_0000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultimodalEmbeddingAnswer {
    pub response: EmbeddingResponse,
    pub details: MediaPromptTokensDetails,
}

pub fn decode_multimodal_embedding(bytes: &[u8]) -> Result<Json, MultimodalEmbeddingError> {
    if bytes.len() > RESPONSE_BODY_LIMIT {
        return Err(MultimodalEmbeddingError::ResponseTooLarge {
            size: bytes.len(),
            limit: RESPONSE_BODY_LIMIT,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| MultimodalEmbeddingError::Decode { source })
}

pub fn transform_response(
    status: u16,
    body: &[u8],
    request: &Json,
    model: &str,
) -> Result<MultimodalEmbeddingAnswer, MultimodalEmbeddingError> {
    if status != 200 {
        return Err(MultimodalEmbeddingError::Status { status });
    }
    let decoded = decode_multimodal_embedding(body)?;
    let predictions = predictions(&decoded)?;
    let data = records(predictions)?;
    let details = calculate_usage(request, predictions)?;
    Ok(MultimodalEmbeddingAnswer {
        response: EmbeddingResponse {
            object: String::from("list"),
            data,
            model: model.to_owned(),
            usage: EmbeddingUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                completion_tokens_details: None,
                prompt_tokens_details: None,
                total_tokens: 0,
            },
        },
        details,
    })
}

fn predictions(response: &Json) -> Result<&[Json], MultimodalEmbeddingError> {
    let Json::Object(fields) = response else {
        return Err(MultimodalEmbeddingError::MissingPredictions);
    };
    let Some(value) = fields.get("predictions") else {
        return Err(MultimodalEmbeddingError::MissingPredictions);
    };
    let Json::List(items) = value else {
        return Err(MultimodalEmbeddingError::PredictionsNotList);
    };
    Ok(items)
}

fn records(predictions: &[Json]) -> Result<Vec<EmbeddingRecord>, MultimodalEmbeddingError> {
    let mut records = Vec::new();
    for (index, prediction) in predictions.iter().enumerate() {
        match prediction {
            Json::Object(fields) if !fields.is_empty() => {
                collect_prediction(fields, index, &mut records)?;
            }
            Json::Object(_) | Json::Null => {}
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                return Err(MultimodalEmbeddingError::PredictionNotObject { index });
            }
        }
    }
    Ok(records)
}

fn collect_prediction(
    fields: &Extra,
    index: usize,
    records: &mut Vec<EmbeddingRecord>,
) -> Result<(), MultimodalEmbeddingError> {
    let record_index =
        u32::try_from(index).map_err(|_source| MultimodalEmbeddingError::IndexOverflow)?;
    if let Some(vector) = fields.get("textEmbedding") {
        records.push(record(vector, record_index, "textEmbedding", index)?);
        return Ok(());
    }
    if let Some(vector) = fields.get("imageEmbedding") {
        records.push(record(vector, record_index, "imageEmbedding", index)?);
        return Ok(());
    }
    if let Some(segments) = fields.get("videoEmbeddings") {
        collect_segments(segments, record_index, index, records)?;
    }
    Ok(())
}

fn collect_segments(
    segments: &Json,
    record_index: u32,
    prediction_index: usize,
    records: &mut Vec<EmbeddingRecord>,
) -> Result<(), MultimodalEmbeddingError> {
    let Json::List(items) = segments else {
        return Err(MultimodalEmbeddingError::NotVector {
            field: "videoEmbeddings",
            index: prediction_index,
        });
    };
    for segment in items {
        let Json::Object(fields) = segment else {
            return Err(MultimodalEmbeddingError::PredictionNotObject {
                index: prediction_index,
            });
        };
        let Some(embedding) = fields.get("embedding") else {
            return Err(MultimodalEmbeddingError::MissingSegmentEmbedding {
                index: prediction_index,
            });
        };
        records.push(record(
            embedding,
            record_index,
            "embedding",
            prediction_index,
        )?);
    }
    Ok(())
}

fn record(
    vector: &Json,
    index: u32,
    field: &'static str,
    prediction: usize,
) -> Result<EmbeddingRecord, MultimodalEmbeddingError> {
    Ok(EmbeddingRecord {
        object: String::from("embedding"),
        embedding: EmbeddingVector::Floats(numbers(vector, field, prediction)?),
        index,
    })
}

fn numbers(
    value: &Json,
    field: &'static str,
    index: usize,
) -> Result<Vec<serde_json::Number>, MultimodalEmbeddingError> {
    let Json::List(items) = value else {
        return Err(MultimodalEmbeddingError::NotVector { field, index });
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Json::Number(number) = item else {
            return Err(MultimodalEmbeddingError::NotVector { field, index });
        };
        values.push(number.clone());
    }
    Ok(values)
}
