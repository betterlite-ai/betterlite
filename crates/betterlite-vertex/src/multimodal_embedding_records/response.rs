use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingRecord;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::embedding::EmbeddingUsage;
use betterlite_openai::embedding::EmbeddingVector;

use super::error::MultimodalEmbeddingError as Error;
use super::usage::MediaPromptTokensDetails;
use super::usage::calculate_usage;

pub const RESPONSE_BODY_LIMIT: usize = 0x0200_0000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultimodalEmbeddingAnswer {
    pub response: EmbeddingResponse,
    pub details: MediaPromptTokensDetails,
}

pub fn decode_multimodal_embedding(bytes: &[u8]) -> Result<Json, Error> {
    if bytes.len() > RESPONSE_BODY_LIMIT {
        return Err(Error::ResponseTooLarge {
            size: bytes.len(),
            limit: RESPONSE_BODY_LIMIT,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| Error::Decode { source })
}

pub fn transform_response(
    status: u16,
    body: &[u8],
    request: &Json,
    model: &str,
) -> Result<MultimodalEmbeddingAnswer, Error> {
    if status != 200 {
        return Err(Error::Status { status });
    }
    let decoded = decode_multimodal_embedding(body)?;
    let Json::Object(fields) = &decoded else {
        return Err(Error::MissingPredictions);
    };
    let predictions = match fields.get("predictions") {
        None => return Err(Error::MissingPredictions),
        Some(Json::List(items)) => items.as_slice(),
        Some(_) => return Err(Error::PredictionsNotList),
    };
    let data = records(predictions)?;
    let details = calculate_usage(request, predictions)?;
    let response = EmbeddingResponse {
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
    };
    Ok(MultimodalEmbeddingAnswer { response, details })
}

fn records(predictions: &[Json]) -> Result<Vec<EmbeddingRecord>, Error> {
    let mut records = Vec::new();
    for (index, prediction) in predictions.iter().enumerate() {
        let Json::Object(fields) = prediction else {
            if matches!(prediction, Json::Null) {
                continue;
            }
            return Err(Error::PredictionNotObject { index });
        };
        if fields.is_empty() {
            continue;
        }
        let record_index = u32::try_from(index).map_err(|source| Error::IntegerRange {
            what: "index",
            source,
        })?;
        if let Some(vector) = fields.get("textEmbedding") {
            records.push(record(vector, record_index, "textEmbedding", index)?);
            continue;
        }
        if let Some(vector) = fields.get("imageEmbedding") {
            records.push(record(vector, record_index, "imageEmbedding", index)?);
            continue;
        }
        if let Some(segments) = fields.get("videoEmbeddings") {
            records.extend(segment_records(segments, record_index, index)?);
        }
    }
    Ok(records)
}

fn segment_records(
    value: &Json,
    record_index: u32,
    prediction: usize,
) -> Result<Vec<EmbeddingRecord>, Error> {
    let Json::List(items) = value else {
        return Err(Error::Field("videoEmbeddings", "list", prediction));
    };
    let mut records = Vec::with_capacity(items.len());
    for segment in items {
        let Json::Object(fields) = segment else {
            return Err(Error::PredictionNotObject { index: prediction });
        };
        let embedding =
            fields
                .get("embedding")
                .ok_or(Error::Field("embedding", "present", prediction))?;
        records.push(record(embedding, record_index, "embedding", prediction)?);
    }
    Ok(records)
}

fn record(
    vector: &Json,
    index: u32,
    field: &'static str,
    prediction: usize,
) -> Result<EmbeddingRecord, Error> {
    let Json::List(items) = vector else {
        return Err(Error::Field(field, "list of numbers", prediction));
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Json::Number(number) = item else {
            return Err(Error::Field(field, "list of numbers", prediction));
        };
        values.push(number.clone());
    }
    Ok(EmbeddingRecord {
        object: String::from("embedding"),
        embedding: EmbeddingVector::Floats(values),
        index,
    })
}
