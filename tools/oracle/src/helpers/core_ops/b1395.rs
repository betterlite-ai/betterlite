use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::vertex_embedding_dispatch::Defaults;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_provider::vertex_embedding_binding::arguments::{
    EmbedContentArguments, MultimodalArguments, TextArguments,
};
use betterlite_provider::vertex_embedding_binding::{
    VertexEmbeddingError, VertexEmbeddingHandlers, VertexEmbeddingPrepared, VertexEmbeddingRequest,
    execute, prepare,
};
use betterlite_vertex::gemini_embedding_call::error::GeminiEmbeddingCallError;
use betterlite_vertex::gemini_embedding_records::response::EmbeddingReply;
use betterlite_vertex::multimodal_embedding_records::response::MultimodalEmbeddingAnswer;
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
        Ok(match &prepared {
            VertexEmbeddingPrepared::EmbedContent(call) => embed_content_view(call),
            VertexEmbeddingPrepared::Multimodal(call) => multimodal_view(call),
            VertexEmbeddingPrepared::Text(call) => text_view(call),
        })
    }
}

fn request(input: &Input) -> VertexEmbeddingRequest {
    VertexEmbeddingRequest {
        model: input.model.clone(),
        input: EmbeddingInput::Text(input.input.clone()),
        optional_params: input.optional_params.clone(),
        litellm_params: input.litellm_params.clone(),
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: input.api_base.clone(),
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

fn text_remaining(params: &Json) -> Vec<&str> {
    match params {
        Json::Object(fields) => remaining(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Vec::new(),
    }
}

fn embed_content_view(call: &EmbedContentArguments) -> Value {
    json!({
        "arm": "embed_content",
        "project": call.project,
        "location": call.location,
        "credentials": call.credentials.as_ref().and_then(ConfiguredCredentials::as_text),
        "api_base": call.api_base,
        "remaining": remaining(&call.optional_params),
        "extra_headers": call.extra_headers,
        "api_key_set": false,
        "timeout": null,
        "litellm_params_set": false,
        "status": null,
    })
}

fn multimodal_view(call: &MultimodalArguments) -> Value {
    json!({
        "arm": "multimodal",
        "project": call.project,
        "location": call.location,
        "credentials": call.credentials.as_ref().and_then(ConfiguredCredentials::as_text),
        "api_base": call.api_base,
        "remaining": remaining(&call.optional_params),
        "extra_headers": null,
        "api_key_set": false,
        "timeout": null,
        "litellm_params_set": call.litellm_params.is_some(),
        "status": null,
    })
}

fn text_view(call: &TextArguments) -> Value {
    json!({
        "arm": "text",
        "project": call.project,
        "location": call.location,
        "credentials": call.credentials.as_ref().and_then(ConfiguredCredentials::as_text),
        "api_base": call.api_base,
        "remaining": text_remaining(&call.optional_params),
        "extra_headers": null,
        "api_key_set": call.api_key.is_some(),
        "timeout": call.timeout.map(Duration::as_secs),
        "litellm_params_set": call.litellm_params.is_some(),
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
    Ok(match prepared {
        VertexEmbeddingPrepared::EmbedContent(_) => {
            json!({"arm": "embed_content", "status": status})
        }
        VertexEmbeddingPrepared::Multimodal(_) => json!({"arm": "multimodal", "status": status}),
        VertexEmbeddingPrepared::Text(_) => json!({"arm": "text", "status": status}),
    })
}

// the three handlers, refusing every arm the way main.py's provider does
struct Failing;

impl VertexEmbeddingHandlers for Failing {
    fn embed_content<'call>(
        &'call self,
        _call: &'call EmbedContentArguments,
    ) -> impl Future<Output = Result<EmbeddingReply, GeminiEmbeddingCallError>> + Send + 'call {
        async move { Err(GeminiEmbeddingCallError::MissingApiKey) }
    }

    fn multimodal<'call>(
        &'call self,
        _call: &'call MultimodalArguments,
    ) -> impl Future<Output = Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError>> + Send + 'call
    {
        async move {
            Err(VertexEmbeddingCallError::Status {
                status: FAIL_STATUS,
                body: b"the provider refused the request".to_vec(),
            })
        }
    }

    fn text<'call>(
        &'call self,
        _call: &'call TextArguments,
    ) -> impl Future<Output = Result<EmbeddingResponse, VertexEmbeddingCallError>> + Send + 'call
    {
        async move {
            Err(VertexEmbeddingCallError::Status {
                status: FAIL_STATUS,
                body: b"the provider refused the request".to_vec(),
            })
        }
    }
}
