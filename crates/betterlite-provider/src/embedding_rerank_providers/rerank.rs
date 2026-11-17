use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::cohere_rerank_settings;
use betterlite_config::sections::embedding_provider_settings::Settings;
use betterlite_config::sections::jina_infinity_rerank;
use betterlite_core::embedding_rerank_providers::{RerankConfig, rerank_config};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_openai::cohere_rerank::{self, RerankVersion};
use betterlite_openai::host_rerank::{self, HostProvider};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::voyage_rerank;
use secrecy::SecretString;
use uuid::Uuid;

use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, OptionalRerankParams, Params, ReturnDocuments,
    get_optional_rerank_params,
};
use crate::embedding_rerank_providers::error::FamilyError;
use crate::embedding_rerank_providers::request::{encode, http_headers, url};
use crate::embedding_rerank_providers::{Call, Decode, EmbeddingRerankPrepared};

mod config;
mod voyage;

// a rerank answer is small next to an embedding batch, and these are the limits the host leaves use
const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0080_0000,
    max_event_bytes: 0x0080_0000,
    max_items: 200_000,
};

pub struct RerankCallInput<'request> {
    pub provider: ProviderId,
    pub model: &'request str,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    pub top_n: Option<i64>,
    pub rank_fields: Option<&'request [String]>,
    pub return_documents: ReturnDocuments,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub headers: Option<&'request Headers>,
    pub cohere: &'request cohere_rerank_settings::Settings,
    pub host: &'request jina_infinity_rerank::Settings,
    pub settings: &'request Settings,
    pub environment: &'request (dyn EnvironmentLookup + Send + Sync),
    pub timeout: Duration,
}

// LiteLLM picks the Cohere client from the parameters the caller actually set
pub struct FamilyRerankConfig<'settings> {
    pub config: RerankConfig,
    pub cohere: &'settings cohere_rerank_settings::Settings,
    pub host: &'settings jina_infinity_rerank::Settings,
    pub environment: &'settings (dyn EnvironmentLookup + Send + Sync),
}

pub fn prepare(input: &RerankCallInput<'_>) -> Result<EmbeddingRerankPrepared, FamilyError> {
    let present = present_version_params(input.max_chunks_per_doc, input.max_tokens_per_doc);
    let family = family_config(input, &present)?;
    let config = family.config;
    let mut non_default = non_default_params(input);
    let params = get_optional_rerank_params(&family, optional_params(input, &mut non_default))?;
    let headers = family_headers(&family, input, &params)?;
    let endpoint = family.url(input.api_base, input.model)?;
    let body = family.transform_rerank_request(input.model, &params, &headers, None)?;
    Ok(EmbeddingRerankPrepared(Call {
        url: url(family_name(config), &endpoint)?,
        headers: http_headers(&headers)?,
        body: encode(&body)?,
        timeout: input.timeout,
        decode: Decode::Rerank(config),
    }))
}

// LiteLLM picks the client from the params the caller set, before it maps them into the body
fn family_config<'settings>(
    input: &'settings RerankCallInput<'settings>,
    present: &[String],
) -> Result<FamilyRerankConfig<'settings>, FamilyError> {
    let config =
        rerank_config(input.provider, input.api_base, present).ok_or(FamilyError::Unsupported {
            provider: input.provider.to_string(),
        })?;
    Ok(FamilyRerankConfig {
        config,
        cohere: input.cohere,
        host: input.host,
        environment: input.environment,
    })
}

fn optional_params<'request>(
    input: &'request RerankCallInput<'_>,
    non_default: &'request mut Params,
) -> OptionalRerankParams<'request> {
    OptionalRerankParams {
        model: input.model,
        drop_params: false,
        query: input.query,
        documents: input.documents,
        // the slug LiteLLM passes as custom_llm_provider, which its Cohere arm reads
        custom_llm_provider: Some(<&'static str>::from(input.provider)),
        top_n: input.top_n,
        rank_fields: input.rank_fields,
        return_documents: input.return_documents,
        max_chunks_per_doc: input.max_chunks_per_doc,
        max_tokens_per_doc: input.max_tokens_per_doc,
        instruction: input.instruction,
        non_default_params: Some(non_default),
    }
}

// LiteLLM adds rank_fields to the named arguments only, and the Cohere arm reads it from this map
fn non_default_params(input: &RerankCallInput<'_>) -> Params {
    let mut non_default = Params::new();
    if let Some(rank_fields) = input.rank_fields {
        non_default.insert(
            String::from("rank_fields"),
            Json::List(rank_fields.iter().cloned().map(Json::Text).collect()),
        );
    }
    non_default
}

// `headers = headers or litellm.headers or {}` before the family arm, which may replace or extend them
fn family_headers(
    family: &FamilyRerankConfig<'_>,
    input: &RerankCallInput<'_>,
    params: &Params,
) -> Result<Headers, FamilyError> {
    let supplied = input
        .settings
        .resolve_headers(input.headers, &Headers::new());
    family.validate_environment(EnvironmentRequest {
        headers: &supplied,
        model: input.model,
        api_key: input.api_key,
        optional_params: Some(params),
        litellm_params: None,
    })
}

const fn family_name(config: RerankConfig) -> &'static str {
    match config {
        RerankConfig::Cohere | RerankConfig::CohereV2 => "cohere",
        RerankConfig::Infinity => "infinity",
        RerankConfig::JinaAi => "jina",
        RerankConfig::Voyage => "voyage",
    }
}

fn present_version_params(
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
) -> Vec<String> {
    let mut present = Vec::new();
    if max_chunks_per_doc.is_some() {
        present.push(String::from("max_chunks_per_doc"));
    }
    if max_tokens_per_doc.is_some() {
        present.push(String::from("max_tokens_per_doc"));
    }
    present
}

impl FamilyRerankConfig<'_> {
    // the contract's get_complete_url has no error path, so preparation calls this one to keep the failure
    pub fn url(&self, api_base: Option<&str>, _model: &str) -> Result<String, FamilyError> {
        match self.config {
            RerankConfig::Cohere => Ok(cohere_rerank::complete_url(RerankVersion::V1, api_base)),
            RerankConfig::CohereV2 => Ok(cohere_rerank::complete_url(RerankVersion::V2, api_base)),
            RerankConfig::Infinity => Ok(host_rerank::infinity_complete_url(api_base)?),
            RerankConfig::JinaAi => Ok(host_rerank::jina_complete_url(api_base)?),
            RerankConfig::Voyage => Ok(voyage_rerank::endpoint(api_base)),
        }
    }
}

// a rerank answer from any of the four families, in the shared rerank response records
pub fn decode(
    config: RerankConfig,
    status: u16,
    body: &[u8],
) -> Result<RerankResponse, FamilyError> {
    let response = match config {
        RerankConfig::Cohere => cohere_rerank::transform_rerank_response(status, body, LIMITS)?,
        RerankConfig::CohereV2 => cohere_rerank::transform_rerank_response(status, body, LIMITS)?,
        RerankConfig::Infinity => host_rerank::decode_host_rerank(
            HostProvider::Infinity,
            status,
            body,
            Uuid::new_v4,
            |_| {},
        )?,
        RerankConfig::JinaAi => {
            host_rerank::decode_host_rerank(HostProvider::Jina, status, body, Uuid::new_v4, |_| {})?
        }
        RerankConfig::Voyage => voyage::voyage_response(status, body)?,
    };
    Ok(response)
}
