use std::collections::BTreeSet;
use std::error::Error;

use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use crate::catalog::ModelCatalog;
use reply::{OllamaMetadataError, ShowReply, decode_ollama_model_reply};

pub mod reply;

mod tests;

pub const DEFAULT_API_BASE: &str = "http://localhost:11434";
pub const PROVIDER: &str = "ollama";
pub const MODE: &str = "chat";
const SHOW_PATH: &str = "/api/show";
const OLLAMA_PREFIX: &str = "ollama/";
const OLLAMA_CHAT_PREFIX: &str = "ollama_chat/";
const TOOLS_MARKER: &str = "tools";
const SERVER_SUFFIXES: [&str; 6] = [
    "/api/generate",
    "/api/chat",
    "/api/embed",
    "/api/embeddings",
    "/api/show",
    "/api/tags",
];
#[derive(Clone, Debug, Default)]
pub struct OllamaSettings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct LiteLlmGlobals {
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    // the last get_secret_str step only ever supplies the value the environment did not
    pub secret_manager_api_key: Option<SecretString>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ShowBody {
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ShowRequest {
    pub url: String,
    pub body: ShowBody,
    pub headers: Vec<(&'static str, SecretString)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait OllamaTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post_show(&self, request: &ShowRequest) -> Result<ShowResponse, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RuntimeModelInfo {
    pub key: String,
    pub litellm_provider: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_function_calling: Option<bool>,
    pub input_cost_per_token: Decimal,
    pub output_cost_per_token: Decimal,
    pub max_tokens: Option<i64>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

#[derive(Debug)]
pub struct OllamaModelInfo<'context, Transport> {
    pub settings: &'context OllamaSettings,
    pub globals: &'context LiteLlmGlobals,
    pub catalog: &'context ModelCatalog,
    pub transport: &'context Transport,
}

impl<Transport: OllamaTransport> OllamaModelInfo<'_, Transport> {
    pub fn get_model_info(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> Result<Option<RuntimeModelInfo>, OllamaMetadataError> {
        if self.is_static_ollama_model(model) {
            return Ok(None);
        }
        self.get_runtime_model_info(model, api_base, api_key)
            .map(Some)
    }

    pub fn get_runtime_model_info(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> Result<RuntimeModelInfo, OllamaMetadataError> {
        let stripped = strip_ollama_model_prefix(model);
        let request = self.show_request(&stripped, api_base, api_key);
        let response = match self.transport.post_show(&request) {
            Ok(response) if (200..300).contains(&response.status) => response,
            Ok(_) | Err(_) => {
                tracing::debug!("OllamaError: Could not get model info.");
                return Ok(RuntimeModelInfo {
                    key: stripped,
                    litellm_provider: PROVIDER.to_owned(),
                    mode: MODE.to_owned(),
                    supports_function_calling: None,
                    input_cost_per_token: Decimal::ZERO,
                    output_cost_per_token: Decimal::ZERO,
                    max_tokens: None,
                    max_input_tokens: None,
                    max_output_tokens: None,
                });
            }
        };
        let reply = decode_ollama_model_reply(&response.body)?;
        Ok(runtime_info(stripped, &reply))
    }

    fn show_request(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> ShowRequest {
        let suppressed =
            api_base.is_some() && api_key.is_none_or(|key| key.expose_secret().is_empty());
        let authorization = if suppressed {
            None
        } else {
            self.selected_key(api_key)
        };
        let headers = authorization.map_or_else(Vec::new, |key| {
            vec![(
                "Authorization",
                SecretString::from(format!("Bearer {}", key.expose_secret())),
            )]
        });
        ShowRequest {
            url: format!("{}{SHOW_PATH}", self.server_api_base(api_base)),
            body: ShowBody {
                name: model.to_owned(),
            },
            headers,
        }
    }

    fn selected_key(&self, explicit: Option<&SecretString>) -> Option<SecretString> {
        let candidates = [
            explicit,
            self.settings.api_key.as_ref(),
            self.globals.api_key.as_ref(),
            self.globals.openai_key.as_ref(),
            self.globals.secret_manager_api_key.as_ref(),
        ];
        candidates
            .into_iter()
            .flatten()
            .find(|key| !key.expose_secret().is_empty())
            .cloned()
    }

    fn server_api_base(&self, explicit: Option<&str>) -> String {
        let configured = [
            explicit.unwrap_or_default(),
            self.settings.api_base.as_deref().unwrap_or_default(),
        ];
        let base = configured
            .into_iter()
            .find(|base| !base.is_empty())
            .unwrap_or(DEFAULT_API_BASE);
        let trimmed = base.trim_end_matches('/');
        for suffix in SERVER_SUFFIXES {
            if let Some(stripped) = trimmed.strip_suffix(suffix) {
                return stripped.to_owned();
            }
        }
        trimmed.to_owned()
    }

    fn is_static_ollama_model(&self, model: &str) -> bool {
        let stripped = strip_ollama_model_prefix(model);
        let candidates = [
            model.to_owned(),
            stripped.clone(),
            format!("{OLLAMA_PREFIX}{stripped}"),
            format!("{OLLAMA_CHAT_PREFIX}{stripped}"),
        ];
        let keys: BTreeSet<String> = self
            .catalog
            .lookup
            .keys()
            .map(|key| key.to_lowercase())
            .collect();
        candidates
            .iter()
            .any(|candidate| keys.contains(&candidate.to_lowercase()))
    }
}

fn runtime_info(key: String, reply: &ShowReply) -> RuntimeModelInfo {
    let max_tokens = reply.context_length;
    RuntimeModelInfo {
        key,
        litellm_provider: PROVIDER.to_owned(),
        mode: MODE.to_owned(),
        supports_function_calling: Some(reply.template.to_lowercase().contains(TOOLS_MARKER)),
        input_cost_per_token: Decimal::ZERO,
        output_cost_per_token: Decimal::ZERO,
        max_tokens,
        max_input_tokens: max_tokens,
        max_output_tokens: max_tokens,
    }
}

fn strip_ollama_model_prefix(model: &str) -> String {
    model
        .strip_prefix(OLLAMA_PREFIX)
        .or_else(|| model.strip_prefix(OLLAMA_CHAT_PREFIX))
        .unwrap_or(model)
        .to_owned()
}
