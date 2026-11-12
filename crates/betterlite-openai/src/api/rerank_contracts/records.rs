use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::vector_store_records::records::score::Score;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RerankDocument {
    Text(String),
    Object(IndexMap<String, Json>),
}

impl RerankDocument {
    #[must_use]
    pub fn as_json(&self) -> Json {
        match self {
            Self::Text(text) => Json::Text(text.clone()),
            Self::Object(fields) => Json::Object(fields.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TruncationSide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankRequest {
    pub model: String,
    pub query: String,
    pub top_n: Option<i64>,
    pub documents: Vec<RerankDocument>,
    pub rank_fields: Option<Vec<String>>,
    pub return_documents: Option<bool>,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<String>,
    pub truncate_prompt_tokens: Option<i64>,
    pub truncation_side: Option<TruncationSide>,
    pub max_tokens_per_query: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankResponse {
    pub id: Option<String>,
    pub results: Option<Vec<RerankResponseResult>>,
    pub meta: Option<RerankResponseMeta>,
    #[serde(skip)]
    pub hidden_params: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankResponseResult {
    pub index: i64,
    pub relevance_score: Score,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<RerankResponseDocument>,
}

// LiteLLM's result TypedDict keeps the provider's float; converting it here refuses a score the exact decimal cannot hold instead of rounding it.
impl TryFrom<f64> for Score {
    type Error = DecimalLexemeError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        parse_decimal_lexeme(&value.to_string()).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankResponseDocument {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankResponseMeta {
    #[serde(
        default,
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_version: Option<Option<IndexMap<String, Json>>>,
    #[serde(
        default,
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub billed_units: Option<Option<RerankBilledUnits>>,
    #[serde(
        default,
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub tokens: Option<Option<RerankTokens>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankBilledUnits {
    #[serde(
        default,
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub search_units: Option<Option<i64>>,
    #[serde(
        default,
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub total_tokens: Option<Option<i64>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankTokens {
    #[serde(
        default,
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub input_tokens: Option<Option<i64>>,
    #[serde(
        default,
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub output_tokens: Option<Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the rerank response has no field `{name}`")]
pub struct UnknownResponseField {
    pub name: String,
}

impl RerankResponse {
    #[must_use]
    fn field(&self, name: &str) -> Option<Json> {
        let encoded = serde_json::to_value(self).ok()?;
        let Json::Object(fields) = serde_json::from_value::<Json>(encoded).ok()? else {
            return None;
        };
        fields.get(name).cloned()
    }

    pub fn item(&self, name: &str) -> Result<Json, UnknownResponseField> {
        self.field(name).ok_or_else(|| UnknownResponseField {
            name: name.to_owned(),
        })
    }

    #[must_use]
    #[expect(
        clippy::option_if_let_else,
        reason = "map_or would turn the accessor into a one-line function, which the repository bans"
    )]
    pub fn get(&self, name: &str, default: Json) -> Json {
        if let Some(value) = self.field(name) {
            value
        } else {
            default
        }
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        match name {
            "id" | "results" | "meta" => self.field(name).is_some(),
            _ => false,
        }
    }
}
