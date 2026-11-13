use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::Number;
use serde_json::value::RawValue;

// The response usage mapping LiteLLM reads; unknown keys are ignored like the Mapping it declares.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct UsageObject {
    pub(super) input_tokens: Option<CountValue>,
    pub(super) output_tokens: Option<CountValue>,
    pub(super) cache_creation_input_tokens: Option<CountValue>,
    pub(super) cache_read_input_tokens: Option<CountValue>,
    pub(super) cache_creation: Option<CacheCreationValue>,
    pub(super) iterations: Option<Iterations>,
    pub(super) server_tool_use: Option<ServerToolUseValue>,
    pub(super) output_tokens_details: Option<OutputTokensDetailsValue>,
    pub(super) speed: Option<CountValue>,
    pub(super) service_tier: Option<CountValue>,
    pub(super) inference_geo: Option<CountValue>,
}

// A count position: LiteLLM's guards accept a number, a boolean, text or a container and decide per site.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged, from = "Box<RawValue>")]
pub(super) enum CountValue {
    Number(Number),
    Bool(bool),
    Text(String),
    List { empty: bool },
    Object { empty: bool },
    Null,
}

impl From<Box<RawValue>> for CountValue {
    fn from(raw: Box<RawValue>) -> Self {
        match serde_json::from_str::<Json>(raw.get()) {
            Ok(Json::Number(number)) => Self::Number(number),
            Ok(Json::Bool(flag)) => Self::Bool(flag),
            Ok(Json::Text(text)) => Self::Text(text),
            Ok(Json::List(items)) => Self::List {
                empty: items.is_empty(),
            },
            Ok(Json::Object(entries)) => Self::Object {
                empty: entries.is_empty(),
            },
            Ok(Json::Null) | Err(_) => Self::Null,
        }
    }
}

// The raw list is echoed into the normalized usage unchanged, so it stays a Json passthrough.
#[derive(Debug, Deserialize)]
#[serde(try_from = "Json")]
pub(super) struct Iterations {
    pub(super) raw: Json,
    pub(super) items: Vec<UsageIteration>,
}

impl TryFrom<Json> for Iterations {
    type Error = serde_json::Error;

    fn try_from(raw: Json) -> Result<Self, Self::Error> {
        let Json::List(entries) = &raw else {
            return Err(serde_json::Error::custom(
                "the iterations field is not a list",
            ));
        };
        let items = entries
            .iter()
            .map(iteration_from)
            .collect::<Result<Vec<UsageIteration>, serde_json::Error>>()?;
        Ok(Self { raw, items })
    }
}

fn iteration_from(entry: &Json) -> Result<UsageIteration, serde_json::Error> {
    let Json::Object(_) = entry else {
        return Err(serde_json::Error::custom(
            "an iterations entry is not a JSON object",
        ));
    };
    serde_json::from_str(&serde_json::to_string(entry)?)
}

impl UsageObject {
    pub(super) const fn present_iterations(&self) -> Option<&[UsageIteration]> {
        match &self.iterations {
            Some(iterations) if !iterations.items.is_empty() => Some(iterations.items.as_slice()),
            Some(_) | None => None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct UsageIteration {
    pub(super) input_tokens: Option<CountValue>,
    pub(super) output_tokens: Option<CountValue>,
    pub(super) cache_creation_input_tokens: Option<CountValue>,
    pub(super) cache_read_input_tokens: Option<CountValue>,
    pub(super) cache_creation: Option<CacheCreationValue>,
    pub(super) output_tokens_details: Option<OutputTokensDetailsValue>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct CacheCreation {
    pub(super) ephemeral_5m_input_tokens: Option<CountValue>,
    pub(super) ephemeral_1h_input_tokens: Option<CountValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, from = "Box<RawValue>")]
pub(super) enum CacheCreationValue {
    Mapping(CacheCreation),
    Other,
}

impl From<Box<RawValue>> for CacheCreationValue {
    fn from(raw: Box<RawValue>) -> Self {
        object_mapping(&raw).map_or(Self::Other, Self::Mapping)
    }
}

// AnthropicOutputTokensDetails is extra-allow, so unknown keys are ignored and only thinking_tokens is read.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct AnthropicOutputTokensDetails {
    pub(super) thinking_tokens: Option<CountValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, from = "Box<RawValue>")]
pub(super) enum OutputTokensDetailsValue {
    Mapping(AnthropicOutputTokensDetails),
    Other,
}

impl From<Box<RawValue>> for OutputTokensDetailsValue {
    fn from(raw: Box<RawValue>) -> Self {
        object_mapping(&raw).map_or(Self::Other, Self::Mapping)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct ServerToolUsage {
    pub(super) web_search_requests: Option<CountValue>,
    pub(super) tool_search_requests: Option<CountValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, from = "Box<RawValue>")]
pub(super) enum ServerToolUseValue {
    Mapping(ServerToolUsage),
    Other,
}

impl From<Box<RawValue>> for ServerToolUseValue {
    fn from(raw: Box<RawValue>) -> Self {
        object_mapping(&raw).map_or(Self::Other, Self::Mapping)
    }
}

// serde decodes a struct from a JSON array too, so a mapping guard checks the object shape first.
fn object_mapping<T>(raw: &RawValue) -> Option<T>
where
    T: DeserializeOwned,
{
    match serde_json::from_str::<Json>(raw.get()) {
        Ok(Json::Object(_)) => serde_json::from_str(raw.get()).ok(),
        Ok(_) | Err(_) => None,
    }
}

pub(super) fn decode_usage_object(usage_object: &Json) -> Result<UsageObject, serde_json::Error> {
    let Json::Object(_) = usage_object else {
        return Err(serde_json::Error::custom(
            "the usage object is not a JSON object",
        ));
    };
    let text = serde_json::to_string(usage_object)?;
    let decoded = serde_json::from_str(&text)?;
    Ok(decoded)
}
