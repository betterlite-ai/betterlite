use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::vertex_embedding_dispatch::Defaults;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Extra;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_provider::vertex_embedding_binding::arguments::{
    EmbedContentArguments, MultimodalArguments, TextArguments,
};
use betterlite_provider::vertex_embedding_binding::{
    EmbedContentOutcome, MultimodalOutcome, TextOutcome, VertexEmbeddingError,
    VertexEmbeddingHandlers, VertexEmbeddingInput, VertexEmbeddingPrepared, execute, prepare,
};
use betterlite_vertex::gemini_embedding_call::error::GeminiEmbeddingCallError;
use betterlite_vertex::vertex_embedding_call::error::VertexEmbeddingCallError;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

const FAIL_STATUS: u16 = 429;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    config: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    model: String,
    input: String,
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
    #[serde(default)]
    fail: bool,
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
        if input.fail {
            return failed(&prepared);
        }
        Ok(view(&prepared))
    }
}

fn request(input: &Input) -> VertexEmbeddingInput {
    VertexEmbeddingInput {
        model: input.model.clone(),
        input: EmbeddingInput::Text(input.input.clone()),
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

fn arm(prepared: &VertexEmbeddingPrepared) -> &'static str {
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
        "status": null,
    })
}

fn failed(prepared: &VertexEmbeddingPrepared) -> Result<Value, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let Some(error) = runtime.block_on(execute(prepared, &Failing)).err() else {
        return Err(Box::new(HelperError {
            class: "UnreachableOutcome".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "the failing handlers returned a result".to_owned(),
        }));
    };
    let status = match &error {
        VertexEmbeddingError::EmbedContent(_) => None,
        VertexEmbeddingError::Prediction(error) => Some(error.status_code()),
    };
    Ok(json!({"arm": arm(prepared), "status": status}))
}

// the three handlers, refusing every arm the way main.py's provider does
struct Failing;

fn refused() -> VertexEmbeddingCallError {
    VertexEmbeddingCallError::Status {
        status: FAIL_STATUS,
        body: b"the provider refused the request".to_vec(),
    }
}

impl VertexEmbeddingHandlers for Failing {
    async fn embed_content(&self, _call: &EmbedContentArguments) -> EmbedContentOutcome {
        Err(GeminiEmbeddingCallError::MissingApiKey)
    }

    async fn multimodal(&self, _call: &MultimodalArguments) -> MultimodalOutcome {
        Err(refused())
    }

    async fn text(&self, _call: &TextArguments) -> TextOutcome {
        Err(refused())
    }
}
