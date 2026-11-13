use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::Serializer as Wire;

// the four optional blocks every mapped tool may carry
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Options {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_callers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_examples: Option<Vec<Json>>,
}

impl Options {
    // a hosted tool keeps the caller's fields, so an option overwrites its key the way a dict assignment does
    pub fn apply(self, fields: &mut Object) {
        if let Some(value) = self.cache_control {
            fields.insert("cache_control".to_owned(), value);
        }
        if let Some(value) = self.defer_loading {
            fields.insert("defer_loading".to_owned(), Json::Bool(value));
        }
        if let Some(callers) = self.allowed_callers {
            let list = callers.into_iter().map(Json::Text).collect();
            fields.insert("allowed_callers".to_owned(), Json::List(list));
        }
    }
}

pub const CUSTOM_TYPE: &str = "custom";
pub const ADVISOR_TYPE: &str = "advisor_20260301";
pub const URL_TYPE: &str = "url";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceKind {
    Auto,
    Any,
    Tool,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolChoice {
    #[serde(rename = "type")]
    pub kind: ChoiceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_parallel_tool_use: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolDefinition {
    Function(FunctionTool),
    Computer(ComputerTool),
    Hosted(HostedTool),
    Advisor(AdvisorTool),
    Native(Object),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionTool {
    pub name: String,
    pub input_schema: Json,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eager_input_streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub options: Options,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComputerTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub display_width_px: Json,
    pub display_height_px: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_number: Option<Json>,
    #[serde(flatten)]
    pub options: Options,
}

// every field of a hosted tool except type and name is the caller's, options included, and each key goes out once
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostedTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(flatten)]
    pub fields: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdvisorTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: &'static str,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caching: Option<Json>,
    #[serde(flatten)]
    pub options: Options,
}

// a descriptor goes out on the wire, so it derives no Debug and exposes the token only while serializing
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum McpServerTool {
    Descriptor(Object),
    Mapped(MappedServer),
}

#[derive(Clone, Serialize)]
pub struct MappedServer {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub url: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_configuration: Option<ToolConfiguration>,
    #[serde(skip_serializing_if = "absent", serialize_with = "secret")]
    pub authorization_token: Authorization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolConfiguration {
    pub allowed_tools: Vec<String>,
}

// the token stays a secrecy type here and only the serializer writes it out
#[derive(Clone)]
pub struct Authorization(pub(crate) Option<SecretString>);

#[must_use]
pub const fn absent(authorization: &Authorization) -> bool {
    if authorization.0.is_some() {
        return false;
    }
    true
}

// secrecy serializes nothing by default, so the wire boundary is the one place the token is exposed
fn secret<S>(authorization: &Authorization, state: S) -> Result<S::Ok, S::Error>
where
    S: Wire,
{
    match &authorization.0 {
        Some(token) => state.serialize_str(token.expose_secret()),
        None => state.serialize_none(),
    }
}
