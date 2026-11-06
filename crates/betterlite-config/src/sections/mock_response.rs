pub mod failure;
pub mod selection;

mod tests;

use serde::{Deserialize, Serialize};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const PROMPT_TOKEN_COUNT_ENV: &str = "DEFAULT_MOCK_RESPONSE_PROMPT_TOKEN_COUNT";
const COMPLETION_TOKEN_COUNT_ENV: &str = "DEFAULT_MOCK_RESPONSE_COMPLETION_TOKEN_COUNT";
const DEFAULT_PROMPT_TOKEN_COUNT: u64 = 10;
const DEFAULT_COMPLETION_TOKEN_COUNT: u64 = 20;

// LiteLLM's MockResponse accepts a string, an embedding vector, or a mapping it feeds to ModelResponse
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MockResponse {
    Text(String),
    Numbers(Vec<serde_json::Number>),
    Object(MockChatObject),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MockChatObject {
    pub id: Option<String>,
    pub created: Option<u64>,
    pub model: Option<String>,
    // LiteLLM rewrites object on every ModelResponse, so a given value is accepted and ignored
    pub object: Option<String>,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    pub choices: Option<Vec<MockChatChoice>>,
    pub usage: Option<MockUsage>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MockChatChoice {
    pub index: u32,
    pub message: MockChatMessage,
    pub finish_reason: Option<String>,
}

impl Default for MockChatChoice {
    fn default() -> Self {
        Self {
            index: 0,
            message: MockChatMessage::default(),
            finish_reason: Some(String::from("stop")),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MockChatMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<MockToolCall>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MockToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: MockFunctionCall,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MockFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MockUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl Default for MockChatMessage {
    fn default() -> Self {
        Self {
            role: String::from("assistant"),
            content: None,
            tool_calls: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageDefaults {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl Default for UsageDefaults {
    fn default() -> Self {
        Self {
            prompt_tokens: DEFAULT_PROMPT_TOKEN_COUNT,
            completion_tokens: DEFAULT_COMPLETION_TOKEN_COUNT,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    mock_response: Option<MockResponse>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    // one entry per model_list entry, in file order, so chat_setup can pair it with its deployment
    pub deployments: Vec<Option<MockResponse>>,
    pub usage: UsageDefaults,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .enumerate()
            .map(|(index, deployment)| {
                deployment
                    .litellm_params
                    .mock_response
                    .map(|response| resolve(response, index, lookup))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Settings {
            deployments,
            usage: UsageDefaults {
                prompt_tokens: token_count(lookup, PROMPT_TOKEN_COUNT_ENV)?
                    .unwrap_or(DEFAULT_PROMPT_TOKEN_COUNT),
                completion_tokens: token_count(lookup, COMPLETION_TOKEN_COUNT_ENV)?
                    .unwrap_or(DEFAULT_COMPLETION_TOKEN_COUNT),
            },
        })
    }
}

fn resolve<L>(response: MockResponse, index: usize, lookup: &L) -> Result<MockResponse, ConfigError>
where
    L: EnvironmentLookup,
{
    let MockResponse::Text(text) = response else {
        return Ok(response);
    };
    resolve_string(
        text,
        &format!("model_list.{index}.litellm_params.mock_response"),
        lookup,
    )
    .map(MockResponse::Text)
}

fn token_count<L>(lookup: &L, name: &str) -> Result<Option<u64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(None);
    };
    betterlite_env::number::<u64>(name, &text).map_err(|source| ConfigError::Environment { source })
}
