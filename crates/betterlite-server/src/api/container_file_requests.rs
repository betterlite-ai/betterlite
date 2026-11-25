pub mod ports;
pub mod requests;
mod tests;

use axum::http::HeaderMap;
use axum::http::request::Parts;
use betterlite_core::api::container_ownership::DEFAULT_PROVIDER;
use betterlite_core::api::container_ownership::errors::ContainerOwnershipError;
use betterlite_core::api::request_form::convert::{ConvertError, FileTuple};
use betterlite_core::api::request_form::values::{FormError, FormValue};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_prompts::arguments::json::Object as JsonObject;
use indexmap::IndexMap;
use thiserror::Error;

use super::provider_hints::{provider_from_headers, provider_from_query};
use crate::pre_call::failure::PreCallFailure;

// The two provider sources both processors read, already taken from the request by B10059's accessors
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderHints {
    pub header: Option<String>,
    pub query: Option<String>,
}

impl ProviderHints {
    #[must_use]
    pub fn from_request(parts: &Parts) -> Self {
        Self {
            header: provider_from_headers(&parts.headers),
            query: provider_from_query(parts.uri.query()),
        }
    }
}

// LiteLLM's header or query or "openai" chain: an empty value is not truthy, so it falls through
#[must_use]
pub fn pick_provider(hints: &ProviderHints) -> String {
    let header = hints.header.as_deref().filter(|value| !value.is_empty());
    let query = hints.query.as_deref().filter(|value| !value.is_empty());
    header.or(query).unwrap_or(DEFAULT_PROVIDER).to_owned()
}

// LiteLLM's ordered substring guess on the lowercased id, so the first match wins
#[must_use]
pub fn content_type_for_file_id(file_id: &str) -> &'static str {
    let lowered = file_id.to_lowercase();
    if lowered.contains(".png") || lowered.ends_with("png") {
        return "image/png";
    }
    if lowered.contains(".jpg") || lowered.contains(".jpeg") {
        return "image/jpeg";
    }
    if lowered.contains(".gif") {
        return "image/gif";
    }
    if lowered.contains(".csv") {
        return "text/csv";
    }
    if lowered.contains(".json") {
        return "application/json";
    }
    if lowered.contains(".txt") {
        return "text/plain";
    }
    if lowered.contains(".pdf") {
        return "application/pdf";
    }
    "application/octet-stream"
}

// What the common processing answered: the headers on its response object and a body of bytes only on the content route
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerProcessingReply {
    pub headers: HeaderMap,
    pub body: ContainerProcessingBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerProcessingBody {
    Json(JsonObject),
    Bytes(Vec<u8>),
}

// One value of the upload data: a plain form value, a list of file tuples, or the single tuple the upload call takes
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerFormValue {
    Field(FormValue),
    Tuples(Vec<FileTuple>),
    Single(FileTuple),
}

pub type ContainerFormData = IndexMap<String, ContainerFormValue>;

// The data a processor hands the common processing: the content route's JSON object or the upload's converted form
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerRequestData {
    Json(JsonObject),
    Form(ContainerFormData),
}

// B1101's two form failures, kept apart so a route can name the reader that refused the form
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FormReadError {
    #[error(transparent)]
    Form(#[from] FormError),
    #[error(transparent)]
    Convert(#[from] ConvertError),
}

// The guarded section's failure, handed to the common exception handling exactly once
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RequestFailure {
    #[error(transparent)]
    Process(#[from] PreCallFailure),
    #[error("{route_type} expected bytes, got {got}")]
    NotBytes {
        route_type: &'static str,
        got: &'static str,
    },
}

// Every failure a container file request answers with; the first three are raised before the guarded section
#[derive(Debug, Error)]
pub enum ContainerFileRequestError {
    #[error(transparent)]
    AccessDenied(#[from] ContainerOwnershipError),
    #[error(transparent)]
    Form(#[from] FormReadError),
    #[error("Missing required 'file' field")]
    MissingFileField,
    #[error("{0}")]
    Processing(Box<ProxyException>),
}

// Starlette appends "; charset=utf-8" to every text/* media type when it builds the response, so a route appends it to content_type
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerFileDownload {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub headers: HeaderMap,
}
