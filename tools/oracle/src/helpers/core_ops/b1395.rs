use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::vertex_embedding_dispatch::Defaults;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Extra;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_provider::vertex_embedding_binding::{
    VertexEmbeddingInput, VertexEmbeddingPrepared, prepare,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    config: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    model: String,
    #[serde(rename = "input")]
    text: String,
    #[serde(default)]
    optional_params: Extra,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    litellm_params: Option<Extra>,
    #[serde(default)]
    catalog: Option<Box<RawValue>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let loaded = ConfigLoader {
            lookup: input.environment.clone(),
        }
        .load(&input.config)
        .or_config_error()?;
        let defaults = Defaults {
            vertex: &loaded.sections.vertex_credential_sources,
            embedding: &loaded.sections.vertex_embedding_dispatch,
        };
        let prepared = prepare(request(input), defaults, &catalog(input)?).or_value_error()?;
        Ok(view(&prepared))
    }
}

fn request(input: &Input) -> VertexEmbeddingInput {
    VertexEmbeddingInput {
        model: input.model.clone(),
        input: EmbeddingInput::Text(input.text.clone()),
        optional_params: input.optional_params.clone(),
        litellm_params: input.litellm_params.clone(),
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: input.api_base.clone(),
        pinned: None,
        extra_headers: input.extra_headers.clone(),
        timeout: input.timeout_seconds.map(Duration::from_secs),
        aembedding: false,
    }
}

fn catalog(input: &Input) -> Result<ModelCatalog, Box<HelperError>> {
    let Some(raw) = input.catalog.as_deref() else {
        return Ok(ModelCatalog::default());
    };
    ModelCatalog::parse(raw.get().as_bytes(), UnknownFields::Reject).or_value_error()
}

fn remaining(params: &Extra) -> Vec<&str> {
    let mut names: Vec<&str> = params.keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

const fn arm(prepared: &VertexEmbeddingPrepared) -> &'static str {
    match prepared {
        VertexEmbeddingPrepared::EmbedContent(_) => "embed_content",
        VertexEmbeddingPrepared::Multimodal(_) => "multimodal",
        VertexEmbeddingPrepared::Text(_) => "text",
    }
}

fn view(prepared: &VertexEmbeddingPrepared) -> Value {
    let (resolved, extra_headers, api_key_set, timeout, litellm_params_set) = match prepared {
        VertexEmbeddingPrepared::EmbedContent(call) => (
            &call.resolved,
            json!(call.extra_headers),
            false,
            None,
            false,
        ),
        VertexEmbeddingPrepared::Multimodal(call) => (
            &call.resolved,
            Value::Null,
            false,
            None,
            call.litellm_params.is_some(),
        ),
        VertexEmbeddingPrepared::Text(call) => (
            &call.resolved,
            Value::Null,
            call.api_key.is_some(),
            call.timeout.map(|timeout| timeout.as_secs()),
            call.litellm_params.is_some(),
        ),
    };
    json!({
        "arm": arm(prepared),
        "project": resolved.project,
        "location": resolved.location,
        "credentials": resolved.credentials.as_ref().and_then(ConfiguredCredentials::as_text),
        "api_base": resolved.api_base,
        "remaining": remaining(&resolved.optional_params),
        "extra_headers": extra_headers,
        "api_key_set": api_key_set,
        "timeout": timeout,
        "litellm_params_set": litellm_params_set,
    })
}
