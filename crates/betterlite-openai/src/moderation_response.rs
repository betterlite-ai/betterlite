use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

// the provider names a category "harassment/threatening"; LiteLLM dumps the SDK model, which writes both that key and the field name
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationCategories {
    pub harassment: bool,
    pub harassment_threatening: bool,
    pub hate: bool,
    pub hate_threatening: bool,
    pub illicit: Option<bool>,
    pub illicit_violent: Option<bool>,
    pub self_harm: bool,
    pub self_harm_instructions: bool,
    pub self_harm_intent: bool,
    pub sexual: bool,
    pub sexual_minors: bool,
    pub violence: bool,
    pub violence_graphic: bool,
    #[doc(hidden)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedInputTypes {
    pub harassment: Vec<AppliedInputType>,
    pub harassment_threatening: Vec<AppliedInputType>,
    pub hate: Vec<AppliedInputType>,
    pub hate_threatening: Vec<AppliedInputType>,
    pub illicit: Vec<AppliedInputType>,
    pub illicit_violent: Vec<AppliedInputType>,
    pub self_harm: Vec<AppliedInputType>,
    pub self_harm_instructions: Vec<AppliedInputType>,
    pub self_harm_intent: Vec<AppliedInputType>,
    pub sexual: Vec<AppliedInputType>,
    pub sexual_minors: Vec<AppliedInputType>,
    pub violence: Vec<AppliedInputType>,
    pub violence_graphic: Vec<AppliedInputType>,
    #[doc(hidden)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppliedInputType {
    Text,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryScores {
    pub harassment: ExactScore,
    pub harassment_threatening: ExactScore,
    pub hate: ExactScore,
    pub hate_threatening: ExactScore,
    pub illicit: ExactScore,
    pub illicit_violent: ExactScore,
    pub self_harm: ExactScore,
    pub self_harm_instructions: ExactScore,
    pub self_harm_intent: ExactScore,
    pub sexual: ExactScore,
    pub sexual_minors: ExactScore,
    pub violence: ExactScore,
    pub violence_graphic: ExactScore,
    #[doc(hidden)]
    pub extra: IndexMap<String, Value>,
}

// LiteLLM's score is a Python float; betterlite keeps the provider's exact decimal and writes it as a JSON number
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactScore(pub Decimal);

impl Serialize for ExactScore {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        rust_decimal::serde::arbitrary_precision::serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for ExactScore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        rust_decimal::serde::arbitrary_precision::deserialize(deserializer).map(Self)
    }
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
        check_scores(bytes)?;
        serde_json::from_slice(bytes).map_err(|source| ModerationResponseError::Decode { source })
    }
}

// the typed pass cannot say which score failed, so every numeric score is checked against B204's exact parser first
fn check_scores(bytes: &[u8]) -> Result<(), ModerationResponseError> {
    let body: Value = serde_json::from_slice(bytes)
        .map_err(|source| ModerationResponseError::Decode { source })?;
    let results = body.get("results").and_then(Value::as_array);
    for result in results.into_iter().flatten() {
        let scores = result.get("category_scores").and_then(Value::as_object);
        for (category, score) in scores.into_iter().flatten() {
            let Value::Number(number) = score else {
                continue;
            };
            parse_decimal_lexeme(&number.to_string()).map_err(|source| {
                ModerationResponseError::Decimal {
                    category: category.clone(),
                    source,
                }
            })?;
        }
    }
    Ok(())
}

mod maps;
mod tests;
