use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

// pydantic only checks that these are JSON objects; the ordered raw view keeps their content
#[derive(Deserialize)]
pub struct JsonObject {
    #[serde(flatten)]
    pub entries: IgnoredAny,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRequest {
    pub model: Option<String>,
    pub max_tokens: Option<i64>,
    pub stream: Option<bool>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub top_k: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub metadata: Option<JsonObject>,
    pub messages: Vec<Message>,
    pub system: Option<Content<SystemBlock>>,
    pub tools: Option<Vec<Tool>>,
    pub cache_control: Option<CacheControl>,
    pub thinking: Option<JsonObject>,
    pub tool_choice: Option<JsonObject>,
    pub output_config: Option<JsonObject>,
    pub speed: Option<Speed>,
    pub service_tier: Option<ServiceTier>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: Content<Block>,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Content<Item> {
    Text(String),
    Blocks(Vec<Item>),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text(Text),
    ToolUse(ToolUse),
    ToolResult(ToolResult),
    Thinking(Thinking),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemBlock {
    Text(Text),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultBlock {
    Text { text: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub text: NonBlank,
    pub cache_control: Option<CacheControl>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUse {
    pub id: NonEmpty,
    pub name: NonEmpty,
    pub input: JsonObject,
    pub cache_control: Option<CacheControl>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    pub tool_use_id: NonEmpty,
    pub content: Content<ResultBlock>,
    pub is_error: Option<bool>,
    pub cache_control: Option<CacheControl>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thinking {
    pub thinking: String,
    pub signature: NonEmpty,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: NonEmpty,
    pub description: Option<String>,
    pub input_schema: JsonObject,
    #[serde(rename = "type")]
    pub kind: Option<ToolKind>,
    pub cache_control: Option<CacheControl>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Custom,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CacheControl {
    Ephemeral {
        #[serde(default)]
        ttl: Ttl,
    },
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Ttl {
    #[default]
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

impl Ttl {
    #[must_use]
    pub const fn seconds(self) -> u32 {
        match self {
            Self::FiveMinutes => 300,
            Self::OneHour => 3600,
        }
    }
}

#[derive(PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speed {
    Fast,
    Standard,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTier {
    Auto,
    StandardOnly,
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
pub struct NonEmpty(pub String);

impl TryFrom<String> for NonEmpty {
    type Error = WireError;

    fn try_from(text: String) -> Result<Self, WireError> {
        if text.is_empty() {
            Err(WireError::Empty)
        } else {
            Ok(Self(text))
        }
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
pub struct NonBlank(pub String);

// pydantic's \S pattern runs on the Rust regex crate, whose \s is char::is_whitespace
impl TryFrom<String> for NonBlank {
    type Error = WireError;

    fn try_from(text: String) -> Result<Self, WireError> {
        if text.chars().all(char::is_whitespace) {
            Err(WireError::Blank)
        } else {
            Ok(Self(text))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("the field is empty")]
    Empty,
    #[error("the text has no visible character")]
    Blank,
}
