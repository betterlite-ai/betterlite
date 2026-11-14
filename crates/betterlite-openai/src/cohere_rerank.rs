use std::collections::BTreeMap;

use betterlite_config::sections::cohere_rerank_settings::Settings;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::api::rerank_contracts::records::{RerankRequest, RerankResponse};
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
    api_base.filter(|base| !base.is_empty()).map_or_else(
        || default.to_owned(),
        |base| {
            let trimmed = base.trim_end_matches('/');
            if trimmed.ends_with(suffix) {
                trimmed.to_owned()
            } else {
                format!("{trimmed}{suffix}")
            }
        },
    )
}

pub fn request_headers(
    caller: &Headers,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<Headers, CohereRerankError> {
    let key = settings
        .resolve_api_key(api_key)
        .ok_or(CohereRerankError::MissingApiKey)?;
    let bearer = format!("Bearer {}", key.expose_secret());
    let mut headers = Headers::from(
        [
            ("Authorization", bearer.as_str()),
            ("accept", "application/json"),
            ("content-type", "application/json"),
        ]
        .map(|(name, value)| (name.to_owned(), value.to_owned())),
    );
    headers.extend(caller.clone());
    Ok(headers)
}

#[must_use]
pub fn map_rerank_params(
    version: RerankVersion,
    options: &RerankParams,
    _non_default_params: &RerankParams,
) -> RerankParams {
    let limit = match version {
        RerankVersion::V1 => "max_chunks_per_doc",
        RerankVersion::V2 => "max_tokens_per_doc",
    };
    let mut mapped = RerankParams::from(["query", "documents", "top_n", "rank_fields", limit].map(
        |name| {
            (
                name.to_owned(),
                options.get(name).cloned().unwrap_or(Json::Null),
            )
        },
    ));
    let returned = match options.get("return_documents") {
        None | Some(Json::Null) => Json::Bool(true),
        Some(given) => given.clone(),
    };
    mapped.insert("return_documents".to_owned(), returned);
    mapped
}

pub fn transform_rerank_request(
    version: RerankVersion,
    model: &str,
    params: &RerankParams,
) -> Result<RerankParams, CohereRerankError> {
    for name in ["query", "documents"] {
        if !params.contains_key(name) {
            return Err(CohereRerankError::Required { name });
        }
    }
    let (max_chunks_per_doc, max_tokens_per_doc) = match version {
        RerankVersion::V1 => (field(params, "max_chunks_per_doc")?, None),
        RerankVersion::V2 => (None, field(params, "max_tokens_per_doc")?),
    };
    let request = RerankRequest {
        model: model.to_owned(),
        query: field(params, "query")?,
        top_n: field(params, "top_n")?,
        documents: field(params, "documents")?,
        rank_fields: field(params, "rank_fields")?,
        return_documents: field(params, "return_documents")?,
        max_chunks_per_doc,
        max_tokens_per_doc,
        instruction: None,
        truncate_prompt_tokens: None,
        truncation_side: None,
        max_tokens_per_query: None,
    };
    let mut body = serde_json::to_value(&request)
        .and_then(serde_json::from_value::<RerankParams>)
        .map_err(|source| CohereRerankError::Field {
            name: "request",
            source,
        })?;
    body.retain(|_, value| !matches!(value, Json::Null));
    Ok(body)
}

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
                message: str::from_utf8(body).unwrap_or_default().to_owned(),
            })
        }
        Err(error) => Err(CohereRerankError::Decode(error)),
    }
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn get_error_class(
    error_message: String,
    status_code: u16,
    _headers: &Headers,
) -> ProviderError {
    cohere_error(status_code, error_message)
}

fn field<T>(params: &RerankParams, name: &'static str) -> Result<T, CohereRerankError>
where
    T: DeserializeOwned,
{
    serde_json::to_value(params.get(name))
        .and_then(serde_json::from_value)
        .map_err(|source| CohereRerankError::Field { name, source })
}
