use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

use super::input_failure;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Chat,
    Embedding,
    LemonadeModels,
    LemonadeReply,
    OllamaModels,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    HostedVllm,
    Ollama,
    OllamaChat,
    Lemonade,
    Petals,
    Triton,
    Oobabooga,
    Xinference,
}

impl Provider {
    pub(super) const fn slug(self) -> &'static str {
        match self {
            Self::HostedVllm => "hosted_vllm",
            Self::Ollama => "ollama",
            Self::OllamaChat => "ollama_chat",
            Self::Lemonade => "lemonade",
            Self::Petals => "petals",
            Self::Triton => "triton",
            Self::Oobabooga => "oobabooga",
            Self::Xinference => "xinference",
        }
    }

    // the chat arms main.py dispatches by tag; lemonade is reached by its own function
    pub(super) const fn chat_tag(self) -> Option<ChatConfigTag> {
        match self {
            Self::HostedVllm => Some(ChatConfigTag::HostedVllmChat),
            Self::Ollama => Some(ChatConfigTag::Ollama),
            Self::OllamaChat => Some(ChatConfigTag::OllamaChat),
            Self::Petals => Some(ChatConfigTag::Petals),
            Self::Lemonade | Self::Triton | Self::Oobabooga | Self::Xinference => None,
        }
    }

    pub(super) const fn embedding_id(self) -> Option<ProviderId> {
        match self {
            Self::HostedVllm => Some(ProviderId::HostedVllm),
            Self::Triton => Some(ProviderId::Triton),
            Self::Oobabooga => Some(ProviderId::Oobabooga),
            Self::Ollama => Some(ProviderId::Ollama),
            Self::Xinference => Some(ProviderId::Xinference),
            Self::OllamaChat | Self::Lemonade | Self::Petals => None,
        }
    }
}

// the module-level litellm settings the arms read, as a case names them
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct Globals {
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) ollama_key: Option<String>,
    #[serde(default)]
    pub(super) lemonade_key: Option<String>,
    #[serde(default)]
    pub(super) openai_key: Option<String>,
    #[serde(default)]
    pub(super) secret_manager_api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    pub(super) op: Operation,
    pub(super) provider: Provider,
    #[serde(default)]
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) input: Option<Json>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) stream: Option<bool>,
    #[serde(default)]
    pub(super) optional_params: Option<Json>,
    #[serde(default)]
    pub(super) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) catalog: Vec<String>,
    #[serde(default)]
    pub(super) answer: Option<Answer>,
}

impl Case {
    pub(super) fn optional_params(&self) -> Result<Object, Box<HelperError>> {
        match &self.optional_params {
            None | Some(Json::Null) => Ok(Object::new()),
            Some(value) => Ok(serde_json::from_value::<Object>(serde_json::to_value(
                value,
            )?)?),
        }
    }

    // the embedding arms take a prompt string or a list of them, as LiteLLM's embedding input does
    pub(super) fn embedding_input(&self) -> Result<EmbeddingInput, Box<HelperError>> {
        match &self.input {
            Some(Json::Text(text)) => Ok(EmbeddingInput::Text(text.clone())),
            Some(Json::List(items)) => {
                let mut texts = Vec::with_capacity(items.len());
                for item in items {
                    let Json::Text(text) = item else {
                        return Err(input_failure(String::from(
                            "an embedding input list holds a value that is not a string",
                        )));
                    };
                    texts.push(text.clone());
                }
                Ok(EmbeddingInput::Texts(texts))
            }
            None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
                Err(input_failure(String::from(
                    "the embedding case names no prompt string or list of them",
                )))
            }
        }
    }
}
