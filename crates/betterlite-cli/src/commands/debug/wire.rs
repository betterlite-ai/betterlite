#![expect(
    clippy::disallowed_types,
    reason = "LiteLLM types the request and response of a spend log as JsonValue because they hold whatever body the client sent, so no fixed wire type fits"
)]

use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

#[derive(Debug, Default, Deserialize)]
pub struct ErrorInformation {
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub error_code: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub error_class: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub error_message: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub llm_provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SpendLogMetadata {
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub status: Option<String>,
    #[serde(default)]
    pub error_information: Option<ErrorInformation>,
}

// The proxy stores the metadata as a JSON object or as the text of one.
#[derive(Deserialize)]
#[serde(untagged)]
enum MetadataWire {
    Text(String),
    Object(SpendLogMetadata),
}

impl TryFrom<MetadataWire> for SpendLogMetadata {
    type Error = serde_json::Error;

    fn try_from(wire: MetadataWire) -> Result<Self, Self::Error> {
        match wire {
            MetadataWire::Object(metadata) => Ok(metadata),
            MetadataWire::Text(text) if text.is_empty() => Ok(Self::default()),
            MetadataWire::Text(text) => serde_json::from_str(&text),
        }
    }
}

// The raw text keeps the number's own digits, so the Decimal never passes through a float.
#[derive(Deserialize)]
#[serde(transparent)]
struct SpendWire(Box<RawValue>);

impl TryFrom<SpendWire> for Decimal {
    type Error = DecimalLexemeError;

    fn try_from(wire: SpendWire) -> Result<Self, Self::Error> {
        parse_decimal_lexeme(wire.0.get().trim_matches('"'))
    }
}

#[derive(Debug, Deserialize)]
pub struct SpendLogRow {
    pub request_id: String,
    #[serde(
        default,
        alias = "startTime",
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub start_time: Option<String>,
    #[serde(
        default,
        alias = "endTime",
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub end_time: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub model: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub model_group: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub custom_llm_provider: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub api_base: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub call_type: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::NoneAsEmptyString>>::deserialize"
    )]
    pub status: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::TryFromInto<SpendWire>>>::deserialize"
    )]
    pub spend: Decimal,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull>::deserialize"
    )]
    pub prompt_tokens: u64,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull>::deserialize"
    )]
    pub completion_tokens: u64,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull>::deserialize"
    )]
    pub total_tokens: u64,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::TryFromInto<MetadataWire>>>::deserialize"
    )]
    pub metadata: SpendLogMetadata,
}

impl SpendLogRow {
    // The row's own status wins and the metadata's decides only when the row has none.
    #[must_use]
    pub fn failed(&self) -> bool {
        match (&self.status, &self.metadata.status) {
            (Some(status), _) | (None, Some(status)) => status == "failure",
            (None, None) => false,
        }
    }
}

impl SpendLogRow {
    #[must_use]
    pub fn model_label(&self) -> &str {
        match (&self.model, &self.model_group) {
            (Some(model), _) | (None, Some(model)) => model,
            (None, None) => "?",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SessionLogsPage {
    pub data: Vec<SpendLogRow>,
    pub total: i64,
    pub total_pages: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct PassthroughJson(Value);

impl PassthroughJson {
    #[must_use]
    pub fn text(&self) -> String {
        match &self.0 {
            Value::String(text) => text.clone(),
            other => format!("{other:#}"),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct RequestResponsePayload {
    #[serde(default)]
    pub proxy_server_request: PassthroughJson,
    #[serde(default)]
    pub response: PassthroughJson,
    #[serde(default)]
    pub messages: PassthroughJson,
}

mod tests;
