use std::collections::BTreeMap;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::{HeaderMap, Method};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_with::{DefaultOnError, serde_as};
use thiserror::Error;
use url::Url;

use crate::jinja::{TokenObject, TokenValue};

const MISTRAL_MODEL: &str = "mistralai/Mistral-7B-Instruct-v0.1";
pub(super) const LLAMA3_INSTRUCT_MODEL: &str = "meta-llama/Meta-Llama-3-8B-Instruct";
const DEEPSEEK_MODEL: &str = "deepseek-r1/deepseek-r1-7b-instruct";
const TOKENIZER_CONFIG_FILE: &str = "tokenizer_config.json";
const TEMPLATE_FILES: [&str; 2] = ["chat_template.jinja", "chat_template.jinja2"];
pub(super) const BUILTIN_ENTRIES: usize = 3;
pub(super) const MAX_FETCHED_ENTRIES: usize = 1024;

#[derive(Debug)]
pub struct HubClient {
    pub base_url: String,
    pub client: ProxiedClient,
}

#[derive(Debug)]
pub enum Hub {
    Http(HubClient),
    // the oracle's adapter answers both fetches with failure, as LiteLLM's patched handler does
    Unreachable,
}

#[derive(Debug, Error)]
pub enum HubError {
    #[error("the Hugging Face URL for {model} cannot be built")]
    Url {
        model: String,
        #[source]
        source: url::ParseError,
    },
    #[error("could not fetch {url}")]
    Transport {
        url: String,
        #[source]
        source: TransportError,
    },
    #[error("could not parse {url}")]
    Json {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenizerConfig {
    // the body parsed as a document, or none when it was valid JSON but not an object
    Success(Option<TokenizerDocument>),
    Failure,
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TokenizerDocument {
    #[serde(default)]
    pub chat_template: ChatTemplate,
    // a bos or eos token of another JSON type extracts to an empty string, as LiteLLM's _extract_token_value answers
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    pub bos_token: Option<TokenValue>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnError")]
    pub eos_token: Option<TokenValue>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigBody {
    Document(TokenizerDocument),
    // LiteLLM only reads the parsed body when it is a dict, so any other JSON is a config without a template
    Other(IgnoredAny),
}

// LiteLLM keeps whatever the config holds at chat_template and hands it to the renderer, which fails on anything but text
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ChatTemplate {
    Text(String),
    NotText(IgnoredAny),
    #[default]
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatTemplateFile {
    Success(String),
    Failure,
}

impl HubClient {
    pub(super) async fn tokenizer_config(&self, model: &str) -> Result<TokenizerConfig, HubError> {
        let (url, text) = self.target(model, TOKENIZER_CONFIG_FILE)?;
        let response = self.get(url).await.map_err(|source| HubError::Transport {
            url: text.clone(),
            source,
        })?;
        if response.status != 200 {
            return Ok(TokenizerConfig::Failure);
        }
        let body: ConfigBody = serde_json::from_slice(&response.body)
            .map_err(|source| HubError::Json { url: text, source })?;
        let document = match body {
            ConfigBody::Document(document) => Some(document),
            ConfigBody::Other(_) => None,
        };
        Ok(TokenizerConfig::Success(document))
    }

    pub(super) async fn chat_template_file(&self, model: &str) -> ChatTemplateFile {
        for filename in TEMPLATE_FILES {
            let Ok((url, _)) = self.target(model, filename) else {
                continue;
            };
            let Ok(response) = self.get(url).await else {
                continue;
            };
            if response.status != 200 {
                continue;
            }
            let Ok(chat_template) = String::from_utf8(response.body) else {
                continue;
            };
            return ChatTemplateFile::Success(chat_template);
        }
        ChatTemplateFile::Failure
    }

    fn target(&self, model: &str, filename: &str) -> Result<(Url, String), HubError> {
        let text = format!(
            "{}/{model}/raw/main/{filename}",
            self.base_url.trim_end_matches('/')
        );
        let url = Url::parse(&text).map_err(|source| HubError::Url {
            model: model.to_owned(),
            source,
        })?;
        Ok((url, text))
    }

    async fn get(&self, url: Url) -> Result<UpstreamResponse, TransportError> {
        self.client
            .send_buffered(OutboundRequest {
                method: Method::GET,
                url,
                query: Vec::new(),
                headers: HeaderMap::new(),
                body: RequestBody::Empty,
                deadline: self.client.limits.read_timeout,
            })
            .await
    }
}

impl Hub {
    pub(super) async fn tokenizer_config(&self, model: &str) -> Result<TokenizerConfig, HubError> {
        match self {
            Self::Http(client) => client.tokenizer_config(model).await,
            Self::Unreachable => Ok(TokenizerConfig::Failure),
        }
    }

    pub(super) async fn chat_template_file(&self, model: &str) -> ChatTemplateFile {
        match self {
            Self::Http(client) => client.chat_template_file(model).await,
            Self::Unreachable => ChatTemplateFile::Failure,
        }
    }
}

#[must_use]
pub fn builtin_entries() -> BTreeMap<String, TokenizerConfig> {
    let builtins = [
        (
            MISTRAL_MODEL,
            include_str!("fixtures/mistral_7b_instruct.jinja"),
            TokenValue::Text("<s>".to_owned()),
            TokenValue::Text("</s>".to_owned()),
        ),
        (
            LLAMA3_INSTRUCT_MODEL,
            include_str!("fixtures/meta_llama_3_8b_instruct.jinja"),
            TokenValue::Text("<|begin_of_text|>".to_owned()),
            TokenValue::Text(String::new()),
        ),
        (
            DEEPSEEK_MODEL,
            include_str!("fixtures/deepseek_r1_7b_instruct.jinja"),
            TokenValue::Object(TokenObject {
                content: Some("\u{ff5c}begin\u{2581}of\u{2581}sentence\u{ff5c}".to_owned()),
            }),
            TokenValue::Object(TokenObject {
                content: Some("\u{ff5c}end\u{2581}of\u{2581}sentence\u{ff5c}".to_owned()),
            }),
        ),
    ];
    builtins
        .into_iter()
        .map(|(model, chat_template, bos_token, eos_token)| {
            let document = TokenizerDocument {
                chat_template: ChatTemplate::Text(chat_template.to_owned()),
                bos_token: Some(bos_token),
                eos_token: Some(eos_token),
            };
            (model.to_owned(), TokenizerConfig::Success(Some(document)))
        })
        .collect()
}

mod tests;
