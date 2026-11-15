use serde::{Deserialize, Serialize};

use crate::image_models::params::{Field, Params};

use super::error::MarengoEmbeddingsError;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MarengoEmbeddingItem {
    pub embedding: Option<Vec<VectorItem>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum VectorItem {
    Number(f64),
    Text(String),
    Bool(bool),
}

impl VectorItem {
    fn number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Text(text) => text.trim().parse().ok(),
            Self::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct MarengoInvokeResponse {
    #[serde(default)]
    pub data: Vec<MarengoEmbeddingItem>,
    pub embedding: Option<Vec<VectorItem>>,
    #[serde(default)]
    pub embeddings: Vec<MarengoEmbeddingItem>,
}

impl MarengoInvokeResponse {
    pub fn vectors(&self) -> Result<Vec<Vec<f64>>, MarengoEmbeddingsError> {
        // Pydantic coerces every vector in the record before a fallback is chosen.
        let data = item_vectors(&self.data)?;
        let direct = self.embedding.as_deref().map(vector_values).transpose()?;
        let embeddings = item_vectors(&self.embeddings)?;
        if !self.data.is_empty() {
            return Ok(data);
        }
        if let Some(direct) = direct {
            return Ok(vec![direct]);
        }
        Ok(embeddings)
    }
}

fn item_vectors(items: &[MarengoEmbeddingItem]) -> Result<Vec<Vec<f64>>, MarengoEmbeddingsError> {
    let mut vectors = Vec::new();
    for item in items {
        if let Some(embedding) = &item.embedding {
            vectors.push(vector_values(embedding)?);
        }
    }
    Ok(vectors)
}

fn vector_values(items: &[VectorItem]) -> Result<Vec<f64>, MarengoEmbeddingsError> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        values.push(item.number().ok_or(MarengoEmbeddingsError::VectorValue)?);
    }
    Ok(values)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptTokensDetails {
    pub query_count: Option<u64>,
    pub image_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    pub model: String,
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
        for vector in response.vectors()? {
            data.push(EmbeddingRecord {
                embedding: vector,
                index: data.len(),
                object: "embedding",
            });
        }
    }
    Ok(MarengoEmbeddingResponse {
        data,
        model: model.to_owned(),
        usage: billed_usage(batch_data)?,
        invocation_arn: None,
    })
}

#[must_use]
pub fn transform_async_response(response: &Params, model: &str) -> MarengoEmbeddingResponse {
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
        model: model.to_owned(),
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
            let has_text = multi_input.is_some_and(|input| {
                input
                    .input_text
                    .as_deref()
                    .is_some_and(|text| !text.is_empty())
            });
            let images = u64::try_from(multi_input.map_or(0, |input| input.media_sources.len()))
                .map_err(|_unparsed| MarengoEmbeddingsError::UsageOverflow)?;
            (u64::from(has_text), images)
        }
        Some(BilledInputType::Video | BilledInputType::Audio) | None => (0, 0),
    })
}
