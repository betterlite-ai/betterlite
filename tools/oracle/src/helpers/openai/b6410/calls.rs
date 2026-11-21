use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use betterlite_openai::chat::{ChatMessage, Tool};
use betterlite_openai::edenai_chat::ReasoningLookup;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) supported: Vec<SupportedCall>,
    #[serde(default)]
    pub(super) params: Vec<ParamsCall>,
    #[serde(default)]
    pub(super) moonshot_urls: Vec<UrlCall>,
    #[serde(default)]
    pub(super) moonshot_requests: Vec<RequestCall>,
    #[serde(default)]
    pub(super) nadir_cost: Vec<CostCall>,
    #[serde(default)]
    pub(super) novita_headers: Vec<NovitaCall>,
    #[serde(default)]
    pub(super) zai_cache: Vec<CacheCall>,
    #[serde(default)]
    pub(super) maritalk_errors: Vec<ErrorCall>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    Maritalk,
    Llama,
    Moonshot,
    Morph,
    Nadir,
    Nebius,
    Novita,
    Nscale,
    #[serde(rename = "v0")]
    V0,
    Wandb,
    Zai,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SupportedCall {
    pub(super) provider: Provider,
    pub(super) model: String,
    pub(super) reasoning: bool,
    pub(super) chat_models: Vec<String>,
    pub(super) text_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParamsCall {
    pub(super) provider: Provider,
    pub(super) model: String,
    pub(super) reasoning: bool,
    pub(super) requested: Object,
    pub(super) existing: Object,
    pub(super) chat_models: Vec<String>,
    pub(super) text_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UrlCall {
    pub(super) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestCall {
    pub(super) model: String,
    pub(super) reasoning: bool,
    pub(super) messages: Vec<ChatMessage>,
    pub(super) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostCall {
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NovitaCall {
    pub(super) api_key: Option<String>,
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CacheCall {
    pub(super) model: String,
    pub(super) messages: Vec<ChatMessage>,
    pub(super) tools: Option<Vec<Tool>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ErrorCall {
    pub(super) message: String,
    pub(super) status: u16,
    pub(super) headers: Option<BTreeMap<String, String>>,
}

pub(super) struct Lookup(pub(super) bool);

impl ReasoningLookup for Lookup {
    type Error = Infallible;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Infallible> {
        Ok(self.0)
    }
}

pub(super) fn capabilities(chat_models: &[String], text_models: &[String]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: chat_models.iter().cloned().collect::<BTreeSet<_>>(),
        text_models: text_models.iter().cloned().collect::<BTreeSet<_>>(),
    }
}
