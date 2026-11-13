use std::collections::BTreeMap;
use std::str;

use betterlite_config::sections::cohere_rerank_settings::Settings;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Number;
use thiserror::Error;

use crate::api::rerank_contracts::records::{RerankDocument, RerankRequest, RerankResponse};
use crate::api::rerank_contracts::wire::{
    RerankContract, RerankContractError, RerankRecordKind, decode_rerank_contracts,
};
use crate::api::responses_shapes::wire::{ShapeDecodeError, WireDecodeLimits};
use crate::cohere_streams::cohere_error;
use crate::error::ProviderError;
use crate::openai_base_config::Headers;

mod tests;

pub type RerankParams = BTreeMap<String, Json>;

pub const DEFAULT_V1_URL: &str = "https://api.cohere.ai/v1/rerank";
pub const DEFAULT_V2_URL: &str = "https://api.cohere.ai/v2/rerank";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RerankVersion {
    V1,
    V2,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerankOptions {
    pub query: Option<String>,
    pub documents: Option<Vec<RerankDocument>>,
    pub top_n: Option<i64>,
    pub rank_fields: Option<Vec<String>>,
    pub return_documents: Option<bool>,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<String>,
}

#[derive(Debug, Error)]
pub enum CohereRerankError {
    #[error(
        "the Cohere API key is required: set COHERE_API_KEY, CO_API_KEY or litellm_settings.cohere_key"
    )]
    MissingApiKey,
    #[error("{name} is required for Cohere rerank")]
    Required { name: &'static str },
    #[error("the Cohere rerank option {name} has a value of the wrong type")]
    Field {
        name: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Cohere rerank response is not JSON")]
    ResponseJson { status: u16, message: String },
    #[error("the Cohere rerank response could not be decoded")]
    Decode(#[from] RerankContractError),
    #[error("the rerank decoder returned a request record for a response")]
    UnexpectedRecord,
}

#[must_use]
pub fn complete_url(version: RerankVersion, api_base: Option<&str>) -> String {
    let (default, suffix) = match version {
        RerankVersion::V1 => (DEFAULT_V1_URL, "/v1/rerank"),
        RerankVersion::V2 => (DEFAULT_V2_URL, "/v2/rerank"),
    };
    api_base
        .filter(|base| !base.is_empty())
        .map_or_else(|| default.to_owned(), |base| appended(base, suffix))
}

fn appended(base: &str, suffix: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(suffix) {
        trimmed.to_owned()
    } else {
        format!("{trimmed}{suffix}")
    }
}

pub fn request_headers(
    caller: &Headers,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<Headers, CohereRerankError> {
    let key = settings
        .resolve_api_key(api_key)
        .ok_or(CohereRerankError::MissingApiKey)?;
    let mut headers = Headers::from([
        (
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        ),
        ("accept".to_owned(), "application/json".to_owned()),
        ("content-type".to_owned(), "application/json".to_owned()),
    ]);
    headers.extend(caller.clone());
    Ok(headers)
}

// LiteLLM's mapper never reads non_default_params, and it sends only this version's limit
#[must_use]
pub fn map_rerank_params(
    version: RerankVersion,
    options: &RerankOptions,
    _non_default_params: &RerankParams,
) -> RerankParams {
    let mut mapped = RerankParams::from([
        (
            "query".to_owned(),
            options.query.clone().map_or(Json::Null, Json::Text),
        ),
        (
            "documents".to_owned(),
            options
                .documents
                .as_deref()
                .map_or(Json::Null, |documents| {
                    Json::List(documents.iter().map(RerankDocument::as_json).collect())
                }),
        ),
        (
            "top_n".to_owned(),
            options
                .top_n
                .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
        ),
        (
            "rank_fields".to_owned(),
            options.rank_fields.as_deref().map_or(Json::Null, |fields| {
                Json::List(fields.iter().cloned().map(Json::Text).collect())
            }),
        ),
        // an absent or null return_documents takes the mapper's default of true
        (
            "return_documents".to_owned(),
            Json::Bool(options.return_documents.unwrap_or(true)),
        ),
    ]);
    let (name, limit) = match version {
        RerankVersion::V1 => ("max_chunks_per_doc", options.max_chunks_per_doc),
        RerankVersion::V2 => ("max_tokens_per_doc", options.max_tokens_per_doc),
    };
    mapped.insert(
        name.to_owned(),
        limit.map_or(Json::Null, |number| Json::Number(Number::from(number))),
    );
    mapped
}

pub fn transform_rerank_request(
    version: RerankVersion,
    model: &str,
    params: &RerankParams,
) -> Result<RerankParams, CohereRerankError> {
    let query = params
        .get("query")
        .ok_or(CohereRerankError::Required { name: "query" })?;
    let documents = params
        .get("documents")
        .ok_or(CohereRerankError::Required { name: "documents" })?;
    let (max_chunks_per_doc, max_tokens_per_doc) = match version {
        RerankVersion::V1 => (optional(params, "max_chunks_per_doc")?, None),
        RerankVersion::V2 => (None, optional(params, "max_tokens_per_doc")?),
    };
    let request = RerankRequest {
        model: model.to_owned(),
        query: field("query", query)?,
        top_n: optional(params, "top_n")?,
        documents: field("documents", documents)?,
        rank_fields: optional(params, "rank_fields")?,
        return_documents: optional(params, "return_documents")?,
        max_chunks_per_doc,
        max_tokens_per_doc,
        instruction: None,
        truncate_prompt_tokens: None,
        truncation_side: None,
        max_tokens_per_query: None,
    };
    project(&request)
}

// the reply status is not checked here: Cohere's response transform reads only the body
pub fn transform_rerank_response(
    status: u16,
    body: &[u8],
    limits: WireDecodeLimits,
) -> Result<RerankResponse, CohereRerankError> {
    match decode_rerank_contracts(body, RerankRecordKind::Response, limits) {
        Ok(RerankContract::Response(response)) => Ok(response),
        Ok(RerankContract::Request(_)) => Err(CohereRerankError::UnexpectedRecord),
        Err(RerankContractError::Shape(ShapeDecodeError::Json(_))) => {
            Err(CohereRerankError::ResponseJson {
                status,
                message: response_text(body),
            })
        }
        Err(error) => Err(CohereRerankError::Decode(error)),
    }
}

// CohereError carries only the status and the message, so the response headers are dropped
#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn get_error_class(
    error_message: String,
    status_code: u16,
    _headers: &Headers,
) -> ProviderError {
    cohere_error(status_code, error_message)
}

fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| "the Cohere response is not valid UTF-8".to_owned(),
        str::to_owned,
    )
}

// the pydantic dump drops every field the request left None, so only set options reach the wire
fn project(request: &RerankRequest) -> Result<RerankParams, CohereRerankError> {
    let encoded = serde_json::to_value(request).map_err(|source| CohereRerankError::Field {
        name: "request",
        source,
    })?;
    let mut body = serde_json::from_value::<RerankParams>(encoded).map_err(|source| {
        CohereRerankError::Field {
            name: "request",
            source,
        }
    })?;
    body.retain(|_, value| !matches!(value, Json::Null));
    Ok(body)
}

fn field<T>(name: &'static str, value: &Json) -> Result<T, CohereRerankError>
where
    T: DeserializeOwned,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|source| CohereRerankError::Field { name, source })
}

fn optional<T: DeserializeOwned>(
    params: &RerankParams,
    name: &'static str,
) -> Result<Option<T>, CohereRerankError> {
    match params.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(present) => field(name, present).map(Some),
    }
}
