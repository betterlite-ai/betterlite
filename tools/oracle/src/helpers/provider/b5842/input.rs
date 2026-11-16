use std::collections::BTreeMap;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::rerank_contracts::Headers;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) family: Family,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) step: Step,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Family {
    Embed,
    Rerank,
    Audio,
    Passthrough,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) model: String,
    pub(crate) input: Option<EmbeddingInput>,
    pub(crate) options: Object,
    pub(crate) headers: Headers,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) file: Option<AudioFile>,
    pub(crate) reply: Option<String>,
    pub(crate) status: Option<u16>,
    pub(crate) endpoint: String,
    pub(crate) query: Option<Object>,
    pub(crate) data: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AudioFile {
    pub(crate) filename: String,
    pub(crate) content_type: String,
    pub(crate) content: String,
}
