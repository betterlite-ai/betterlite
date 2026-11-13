use serde::de::{Deserializer, Error as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::image_models::params::{Field, Params};

use super::error::MarengoEmbeddingsError;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MarengoEmbeddingItem {
    #[serde(default, deserialize_with = "optional_vector")]
    pub embedding: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MarengoInvokeResponse {
    #[serde(default)]
    pub data: Vec<MarengoEmbeddingItem>,
    #[serde(default, deserialize_with = "optional_vector")]
    pub embedding: Option<Vec<f64>>,
    #[serde(default)]
    pub embeddings: Vec<MarengoEmbeddingItem>,
}

impl MarengoInvokeResponse {
    #[must_use]
    pub fn vectors(&self) -> Vec<Vec<f64>> {
        if !self.data.is_empty() {
            return self
                .data
                .iter()
                .filter_map(|item| item.embedding.clone())
                .collect();
        }
        if let Some(embedding) = &self.embedding {
            return vec![embedding.clone()];
        }
        self.embeddings
            .iter()
            .filter_map(|item| item.embedding.clone())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BilledInputType {
    Text,
    Image,
    Video,
    Audio,
    TextImage,
    MultiInput,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MarengoBilledMultiInput {
    #[serde(rename = "inputText", default)]
    pub input_text: Option<String>,
    #[serde(rename = "mediaSources", default)]
    pub media_sources: Vec<Params>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MarengoBilledRequest {
    #[serde(rename = "inputType", default)]
    pub input_type: Option<BilledInputType>,
    #[serde(default)]
    pub multi_input: Option<MarengoBilledMultiInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PromptTokensDetails {
    pub query_count: Option<u64>,
    pub image_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarengoUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmbeddingRecord {
    pub embedding: Vec<f64>,
    pub index: usize,
    pub object: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarengoEmbeddingResponse {
    pub data: Vec<EmbeddingRecord>,
    pub model: Option<String>,
    pub usage: MarengoUsage,
    pub invocation_arn: Option<Field>,
}

pub fn transform_response(
    response_list: &[MarengoInvokeResponse],
    model: &str,
    batch_data: Option<&[MarengoBilledRequest]>,
) -> Result<MarengoEmbeddingResponse, MarengoEmbeddingsError> {
    let mut data = Vec::new();
    for response in response_list {
        for vector in response.vectors() {
            data.push(EmbeddingRecord {
                embedding: vector,
                index: data.len(),
                object: "embedding",
            });
        }
    }
    Ok(MarengoEmbeddingResponse {
        data,
        model: Some(model.to_owned()),
        usage: billed_usage(batch_data)?,
        invocation_arn: None,
    })
}

#[must_use]
pub fn transform_async_invoke_response(response: &Params, model: &str) -> MarengoEmbeddingResponse {
    let invocation_arn = response
        .get("invocationArn")
        .cloned()
        .unwrap_or_else(|| Field::Text(String::new()));
    MarengoEmbeddingResponse {
        data: vec![EmbeddingRecord {
            embedding: Vec::new(),
            index: 0,
            object: "embedding",
        }],
        model: Some(model.to_owned()),
        usage: MarengoUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
        },
        invocation_arn: Some(invocation_arn),
    }
}

pub fn billed_usage(
    batch_data: Option<&[MarengoBilledRequest]>,
) -> Result<MarengoUsage, MarengoEmbeddingsError> {
    let mut query_count: u64 = 0;
    let mut image_count: u64 = 0;
    for request in batch_data.unwrap_or_default() {
        let (queries, images) = billed_units(request)?;
        query_count = query_count
            .checked_add(queries)
            .ok_or(MarengoEmbeddingsError::UsageOverflow)?;
        image_count = image_count
            .checked_add(images)
            .ok_or(MarengoEmbeddingsError::UsageOverflow)?;
    }
    let details = (query_count > 0 || image_count > 0).then_some(PromptTokensDetails {
        query_count: (query_count > 0).then_some(query_count),
        image_count: (image_count > 0).then_some(image_count),
    });
    Ok(MarengoUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_tokens_details: details,
    })
}

fn billed_units(request: &MarengoBilledRequest) -> Result<(u64, u64), MarengoEmbeddingsError> {
    Ok(match request.input_type {
        Some(BilledInputType::Text) => (1, 0),
        Some(BilledInputType::Image) => (0, 1),
        Some(BilledInputType::TextImage) => (1, 1),
        Some(BilledInputType::MultiInput) => {
            let multi_input = request.multi_input.as_ref();
            let has_text = multi_input
                .is_some_and(|input| input.input_text.as_deref().is_some_and(|text| !text.is_empty()));
            let images = u64::try_from(multi_input.map_or(0, |input| input.media_sources.len()))
                .map_err(|_unparsed| MarengoEmbeddingsError::UsageOverflow)?;
            (u64::from(has_text), images)
        }
        Some(BilledInputType::Video | BilledInputType::Audio) | None => (0, 0),
    })
}

fn optional_vector<'de, D>(deserializer: D) -> Result<Option<Vec<f64>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    float_vector(&value).map(Some).map_err(D::Error::custom)
}

fn float_vector(value: &Value) -> Result<Vec<f64>, String> {
    let Value::Array(items) = value else {
        return Err("Input should be a valid tuple".to_owned());
    };
    items.iter().map(float_item).collect()
}

fn float_item(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| "Input should be a valid number".to_owned()),
        Value::String(text) => text
            .trim()
            .parse::<f64>()
            .map_err(|_unparsed| "Input should be a valid number".to_owned()),
        Value::Bool(flag) => Ok(if *flag { 1.0 } else { 0.0 }),
        Value::Null | Value::Array(_) | Value::Object(_) => {
            Err("Input should be a valid number".to_owned())
        }
    }
}
