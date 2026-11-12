use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::builders::Field;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub type JsonFields = IndexMap<String, Json>;

// LiteLLM's five content-delta TypedDicts merged into one record; every branch reads a key's presence.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContentDelta {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub partial_json: Option<String>,
    #[serde(default)]
    pub citation: Option<Json>,
    #[serde(default)]
    pub thinking: Option<Json>,
    #[serde(default)]
    pub signature: Option<Json>,
    #[serde(default)]
    pub content: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContentBlockDelta {
    #[serde(rename = "type")]
    pub kind: String,
    pub index: i64,
    pub delta: ContentDelta,
}

// ContentBlockStartToolUse and ContentBlockStartText keep every key optional: the TypedDict constructors never check.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ContentBlockStartPayload {
    #[serde(default)]
    pub index: Option<i64>,
    pub content_block: ContentBlockStartBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContentBlockStartBody {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<Json>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub content: Option<Json>,
    #[serde(default)]
    pub caller: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MessageBlockDelta {
    pub delta: MessageDeltaFields,
    #[serde(default)]
    pub usage: Option<Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MessageDeltaFields {
    pub stop_reason: Option<String>,
    pub container: Option<Json>,
}

pub(crate) fn decode<Record>(value: &Json) -> Result<Record, serde_json::Error>
where
    Record: DeserializeOwned,
{
    let text = serde_json::to_string(value)?;
    let decoded = serde_json::from_str(&text)?;
    Ok(decoded)
}

pub(crate) fn decode_field<Record>(value: &Json) -> Option<Field<Record>>
where
    Record: DeserializeOwned,
{
    let text = serde_json::to_string(value).ok()?;
    let field = serde_json::from_str(&text).ok()?;
    Some(field)
}

pub(crate) fn to_json<Value>(value: &Value) -> Option<Json>
where
    Value: Serialize + ?Sized,
{
    let text = serde_json::to_string(value).ok()?;
    let decoded = serde_json::from_str(&text).ok()?;
    Some(decoded)
}
