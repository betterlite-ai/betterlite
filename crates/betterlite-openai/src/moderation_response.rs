use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModerationResponse {
    pub id: String,
    pub model: String,
    pub results: Vec<ModerationResult>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModerationResult {
    pub categories: ModerationCategories,
    pub category_applied_input_types: AppliedInputTypes,
    pub category_scores: CategoryScores,
    pub flagged: bool,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

// the provider names a category "harassment/threatening"; LiteLLM dumps the SDK model, which writes the field name and keeps the provider's key
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModerationCategories {
    pub harassment: bool,
    #[serde(alias = "harassment/threatening")]
    pub harassment_threatening: bool,
    pub hate: bool,
    #[serde(alias = "hate/threatening")]
    pub hate_threatening: bool,
    pub illicit: Option<bool>,
    #[serde(alias = "illicit/violent")]
    pub illicit_violent: Option<bool>,
    #[serde(alias = "self-harm")]
    pub self_harm: bool,
    #[serde(alias = "self-harm/instructions")]
    pub self_harm_instructions: bool,
    #[serde(alias = "self-harm/intent")]
    pub self_harm_intent: bool,
    pub sexual: bool,
    #[serde(alias = "sexual/minors")]
    pub sexual_minors: bool,
    pub violence: bool,
    #[serde(alias = "violence/graphic")]
    pub violence_graphic: bool,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AppliedInputTypes {
    pub harassment: Vec<AppliedInputType>,
    #[serde(alias = "harassment/threatening")]
    pub harassment_threatening: Vec<AppliedInputType>,
    pub hate: Vec<AppliedInputType>,
    #[serde(alias = "hate/threatening")]
    pub hate_threatening: Vec<AppliedInputType>,
    pub illicit: Vec<AppliedInputType>,
    #[serde(alias = "illicit/violent")]
    pub illicit_violent: Vec<AppliedInputType>,
    #[serde(alias = "self-harm")]
    pub self_harm: Vec<AppliedInputType>,
    #[serde(alias = "self-harm/instructions")]
    pub self_harm_instructions: Vec<AppliedInputType>,
    #[serde(alias = "self-harm/intent")]
    pub self_harm_intent: Vec<AppliedInputType>,
    pub sexual: Vec<AppliedInputType>,
    #[serde(alias = "sexual/minors")]
    pub sexual_minors: Vec<AppliedInputType>,
    pub violence: Vec<AppliedInputType>,
    #[serde(alias = "violence/graphic")]
    pub violence_graphic: Vec<AppliedInputType>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppliedInputType {
    Text,
    Image,
}

// LiteLLM's score is a Python float; betterlite keeps the provider's exact decimal and writes it as a JSON number
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CategoryScores {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub harassment: Decimal,
    #[serde(
        alias = "harassment/threatening",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub harassment_threatening: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub hate: Decimal,
    #[serde(
        alias = "hate/threatening",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub hate_threatening: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub illicit: Decimal,
    #[serde(
        alias = "illicit/violent",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub illicit_violent: Decimal,
    #[serde(alias = "self-harm", with = "rust_decimal::serde::arbitrary_precision")]
    pub self_harm: Decimal,
    #[serde(
        alias = "self-harm/instructions",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub self_harm_instructions: Decimal,
    #[serde(
        alias = "self-harm/intent",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub self_harm_intent: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub sexual: Decimal,
    #[serde(
        alias = "sexual/minors",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub sexual_minors: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub violence: Decimal,
    #[serde(
        alias = "violence/graphic",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub violence_graphic: Decimal,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Error)]
pub enum ModerationResponseError {
    #[error("the provider answer is not a moderation response: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "the category_scores value for {category} is not exactly representable as a Decimal: {source}"
    )]
    Decimal {
        category: String,
        #[source]
        source: DecimalLexemeError,
    },
}

impl ModerationResponse {
    pub fn parse(bytes: &[u8]) -> Result<Self, ModerationResponseError> {
        let provider = maps::ProviderMaps::parse(bytes)?;
        provider.check_scores()?;
        let mut response: Self = serde_json::from_slice(bytes)
            .map_err(|source| ModerationResponseError::Decode { source })?;
        provider.keep_alias_keys(&mut response)?;
        maps::check_input_types(&response)?;
        Ok(response)
    }
}

mod maps;
mod tests;
