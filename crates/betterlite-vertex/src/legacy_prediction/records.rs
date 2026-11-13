use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use serde::Deserialize;

use super::error::PredictionError;

pub const PREDICTION_BODY_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    #[serde(default)]
    pub prompt_token_count: Option<i64>,
    #[serde(default)]
    pub candidates_token_count: Option<i64>,
    #[serde(default)]
    pub total_token_count: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub content: String,
    pub finish_reason: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ReplyMeta {
    pub candidates: Option<Vec<Candidate>>,
    pub usage_metadata: Option<UsageMetadata>,
}

// A prediction is either the SDK's Message object or the JSON value the provider returned
#[derive(Clone, Debug)]
pub enum Prediction {
    Message(ChatMessage),
    Value(Json),
}

#[derive(Clone, Debug)]
pub struct TextReply {
    pub text: String,
    pub meta: ReplyMeta,
}

#[derive(Clone, Debug)]
pub struct PredictionsReply {
    pub predictions: Vec<Prediction>,
    pub meta: ReplyMeta,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyChunk {
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkKind {
    Text,
    Chat,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PredictionBody {
    #[serde(default)]
    predictions: Vec<Json>,
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
}

pub fn decode_legacy_prediction(bytes: &[u8]) -> Result<PredictionsReply, PredictionError> {
    if bytes.len() > PREDICTION_BODY_LIMIT {
        return Err(PredictionError::TooLarge {
            limit: PREDICTION_BODY_LIMIT,
        });
    }
    let body: PredictionBody =
        serde_json::from_slice(bytes).map_err(|source| PredictionError::Decode { source })?;
    Ok(PredictionsReply {
        predictions: body
            .predictions
            .into_iter()
            .map(Prediction::Value)
            .collect(),
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: body.usage_metadata,
        },
    })
}

// The SDK's _parse_text_generation_model_response reads prediction["content"]
pub fn text_reply(reply: &PredictionsReply) -> Result<TextReply, PredictionError> {
    let prediction = first_prediction(reply)?;
    let Prediction::Value(value) = prediction else {
        return Err(PredictionError::WrongFieldType {
            field: "predictions",
        });
    };
    Ok(TextReply {
        text: text_text(value)?,
        meta: reply.meta.clone(),
    })
}

// The SDK's _parse_chat_prediction_response reads prediction["candidates"] and its safetyAttributes list
pub fn chat_reply(reply: &PredictionsReply) -> Result<TextReply, PredictionError> {
    let prediction = first_prediction(reply)?;
    let Prediction::Value(value) = prediction else {
        return Err(PredictionError::WrongFieldType {
            field: "predictions",
        });
    };
    let candidates = candidates_of(value)?;
    let Some(first) = candidates.first() else {
        return Err(PredictionError::MissingPrediction);
    };
    Ok(TextReply {
        text: first.content.clone(),
        meta: ReplyMeta {
            candidates: Some(candidates),
            usage_metadata: reply.meta.usage_metadata.clone(),
        },
    })
}

pub fn text_text(value: &Json) -> Result<String, PredictionError> {
    let Json::Object(fields) = value else {
        return Err(PredictionError::WrongFieldType {
            field: "predictions",
        });
    };
    let Some(Json::Text(text)) = fields.get("content") else {
        return Err(PredictionError::WrongFieldType { field: "content" });
    };
    Ok(text.clone())
}

pub fn chat_text(value: &Json) -> Result<String, PredictionError> {
    let candidates = candidates_of(value)?;
    let Some(first) = candidates.first() else {
        return Err(PredictionError::MissingPrediction);
    };
    Ok(first.content.clone())
}

fn first_prediction(reply: &PredictionsReply) -> Result<&Prediction, PredictionError> {
    reply
        .predictions
        .first()
        .ok_or(PredictionError::MissingPrediction)
}

fn candidates_of(value: &Json) -> Result<Vec<Candidate>, PredictionError> {
    let Json::Object(fields) = value else {
        return Err(PredictionError::WrongFieldType {
            field: "predictions",
        });
    };
    let Some(Json::List(entries)) = fields.get("candidates") else {
        return Err(PredictionError::WrongFieldType {
            field: "candidates",
        });
    };
    let Some(Json::List(safety)) = fields.get("safetyAttributes") else {
        return Err(PredictionError::WrongFieldType {
            field: "safetyAttributes",
        });
    };
    if safety.len() < entries.len() {
        return Err(PredictionError::WrongFieldType {
            field: "safetyAttributes",
        });
    }
    let mut candidates = Vec::with_capacity(entries.len());
    for entry in entries {
        candidates.push(candidate_of(entry)?);
    }
    Ok(candidates)
}

fn candidate_of(value: &Json) -> Result<Candidate, PredictionError> {
    let Json::Object(fields) = value else {
        return Err(PredictionError::WrongFieldType {
            field: "candidates",
        });
    };
    let Some(Json::Text(content)) = fields.get("content") else {
        return Err(PredictionError::WrongFieldType { field: "content" });
    };
    let finish_reason = match fields.get("finishReason") {
        None | Some(Json::Null) => None,
        Some(Json::Text(reason)) => Some(reason.clone()),
        Some(_) => {
            return Err(PredictionError::WrongFieldType {
                field: "finishReason",
            });
        }
    };
    Ok(Candidate {
        content: content.clone(),
        finish_reason,
    })
}

#[must_use]
pub fn strip_output_separator(prediction: &Prediction) -> Prediction {
    match prediction {
        Prediction::Message(message) => Prediction::Message(message.clone()),
        Prediction::Value(Json::Text(text)) => match text.split_once("\nOutput:\n") {
            Some((_prompt, output)) => Prediction::Value(Json::Text(output.to_owned())),
            None => prediction.clone(),
        },
        Prediction::Value(_other) => prediction.clone(),
    }
}

// The source's str(completion_response): a bare string stays as is, and None reads as "None"
#[must_use]
pub fn source_string(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null => String::from("None"),
        Json::Bool(true) => String::from("True"),
        Json::Bool(false) => String::from("False"),
        Json::Number(number) => number.to_string(),
        Json::List(items) => {
            let rendered: Vec<String> = items.iter().map(source_string).collect();
            format!("[{}]", rendered.join(", "))
        }
        Json::Object(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(key, item)| format!("{}: {}", quoted(key), source_string(item)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

fn quoted(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
