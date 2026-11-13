use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::dashscope_embedding::error::DashscopeError;
use betterlite_openai::dashscope_embedding::identity::FamilyIdentity;
use betterlite_openai::dashscope_embedding::reply::decode_dashscope_embedding;
use betterlite_openai::dashscope_embedding::{
    DASHSCOPE_DEFAULT_BASE, DASHSCOPE_MISSING_KEY_MESSAGE, DashscopeEmbeddingConfig,
    map_openai_params,
};
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params,
};
use betterlite_provider::dashscope_common::resolve_rerank_base;
use betterlite_provider::dashscope_rerank::decode::decode_dashscope_rerank_reply;
use betterlite_provider::dashscope_rerank::{DashscopeRerankConfig, DashscopeRerankError};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    EmbeddingParamsAndUrl(EmbeddingParamsAndUrlInput),
    EmbeddingReply(EmbeddingReplyInput),
    RerankBase(RerankBaseInput),
    RerankUrlAndHeaders(RerankUrlAndHeadersInput),
    RerankRequest(RerankRequestInput),
    RerankReply(RerankReplyInput),
}

#[derive(Debug, Deserialize)]
struct EmbeddingParamsAndUrlInput {
    non_default_params: IndexMap<String, Json>,
    optional_params: IndexMap<String, Json>,
    api_base: Option<String>,
    env_base: Option<String>,
    api_key: Option<String>,
    env_key: Option<String>,
    #[serde(default)]
    caller_headers: Headers,
}

#[derive(Debug, Deserialize)]
struct EmbeddingReplyInput {
    body: String,
    status: u16,
    request_model: String,
}

#[derive(Debug, Deserialize)]
struct RerankBaseInput {
    api_base: Option<String>,
    env_value: String,
    default_rerank_base: String,
}

#[derive(Debug, Deserialize)]
struct RerankUrlAndHeadersInput {
    api_base: Option<String>,
    env_value: String,
    default_rerank_url: String,
    api_key: Option<String>,
    env_key: Option<String>,
    #[serde(default)]
    caller_headers: Headers,
}

#[derive(Debug, Deserialize)]
struct RerankRequestInput {
    model: String,
    query: String,
    documents: Vec<RerankDocument>,
    top_n: Option<i64>,
    return_documents: Option<bool>,
    rank_fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RerankReplyInput {
    body: String,
    status: u16,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::EmbeddingParamsAndUrl(inner) => embedding_params_and_url(inner),
            Input::EmbeddingReply(inner) => embedding_reply(inner),
            Input::RerankBase(inner) => rerank_base(inner),
            Input::RerankUrlAndHeaders(inner) => rerank_url_and_headers(inner),
            Input::RerankRequest(inner) => rerank_request(inner),
            Input::RerankReply(inner) => rerank_reply(inner),
        }
    }
}

fn embedding_params_and_url(input: &EmbeddingParamsAndUrlInput) -> HelperResult {
    let config = embedding_config(input.env_key.as_deref(), input.env_base.as_deref());
    let mapped = map_openai_params(&input.non_default_params, input.optional_params.clone());
    let headers = config
        .validate_environment(input.api_key.as_deref(), &input.caller_headers)
        .or_value_error()?;
    Ok(json!({
        "params": serde_json::to_value(mapped)?,
        "url": config.complete_url(input.api_base.as_deref()),
        "headers": serde_json::to_value(headers)?,
    }))
}

fn embedding_reply(input: &EmbeddingReplyInput) -> HelperResult {
    let reply =
        decode_dashscope_embedding(input.body.as_bytes(), input.status, &input.request_model)
            .map_err(|error| dashscope_failure(&error))?;
    Ok(json!({
        "response": serde_json::to_value(reply.response)?,
        "id": reply.id,
    }))
}

fn rerank_base(input: &RerankBaseInput) -> HelperResult {
    let base = resolve_rerank_base(
        input.api_base.as_deref(),
        &input.env_value,
        &input.default_rerank_base,
    );
    Ok(json!({"base": base}))
}

fn rerank_url_and_headers(input: &RerankUrlAndHeadersInput) -> HelperResult {
    let config = rerank_config(
        input.env_key.as_deref(),
        &input.env_value,
        &input.default_rerank_url,
    );
    let key = input
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.clone()));
    let request = EnvironmentRequest {
        headers: &input.caller_headers,
        model: "qwen3-rerank",
        api_key: key.as_ref(),
        optional_params: None,
        litellm_params: None,
    };
    let headers = config
        .validate_environment(request)
        .map_err(rerank_failure)?;
    Ok(json!({
        "url": config.get_complete_url(input.api_base.as_deref(), "qwen3-rerank", None),
        "headers": serde_json::to_value(headers)?,
    }))
}

fn rerank_request(input: &RerankRequestInput) -> HelperResult {
    let config = rerank_config(None, "", "");
    let mut non_default_params = Params::new();
    let params = config
        .map_cohere_rerank_params(MapCohereRerankParams {
            non_default_params: &mut non_default_params,
            model: &input.model,
            drop_params: false,
            query: &input.query,
            documents: &input.documents,
            custom_llm_provider: None,
            top_n: input.top_n,
            rank_fields: input.rank_fields.as_deref(),
            return_documents: input.return_documents,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        })
        .map_err(rerank_failure)?;
    let body = config
        .transform_rerank_request(&input.model, &params, &Headers::new(), None)
        .map_err(rerank_failure)?;
    Ok(json!({
        "params": serde_json::to_value(params)?,
        "body": serde_json::to_value(body)?,
    }))
}

fn rerank_reply(input: &RerankReplyInput) -> HelperResult {
    let response = decode_dashscope_rerank_reply(input.body.as_bytes(), input.status)
        .map_err(|error| dashscope_failure(&error))?;
    Ok(serde_json::to_value(response)?)
}

fn embedding_config(env_key: Option<&str>, env_base: Option<&str>) -> DashscopeEmbeddingConfig {
    DashscopeEmbeddingConfig {
        identity: FamilyIdentity {
            missing_key_message: DASHSCOPE_MISSING_KEY_MESSAGE.to_owned(),
            default_base: DASHSCOPE_DEFAULT_BASE.to_owned(),
            env_base: env_base.map(str::to_owned),
            env_key: env_key.map(str::to_owned),
            fallback_key: None,
            empty_key_is_set: true,
        },
    }
}

fn rerank_config(
    env_key: Option<&str>,
    env_value: &str,
    default_rerank_url: &str,
) -> DashscopeRerankConfig {
    DashscopeRerankConfig {
        identity: embedding_config(env_key, None).identity,
        rerank_env_value: env_value.to_owned(),
        default_rerank_url: default_rerank_url.to_owned(),
    }
}

fn dashscope_failure(error: &DashscopeError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "DashScopeException".to_owned(),
        status: Some(error.status_code),
        error_type: None,
        param: None,
        code: None,
        message: error.message.clone(),
    })
}

fn rerank_failure(error: DashscopeRerankError) -> Box<HelperError> {
    match error {
        DashscopeRerankError::Provider(provider) => dashscope_failure(&provider),
        other => Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: other.to_string(),
        }),
    }
}
