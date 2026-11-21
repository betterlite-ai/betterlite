pub mod backend_call;
pub mod cache_control;
pub mod cleaning;
pub mod error;
pub mod header_metadata;
pub mod provider_headers;
pub mod trace_context;
pub mod user_mapping;

mod tests;

use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_config::sections::request_headers::Settings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_headers::backend_call::{BackendCallArguments, backend_call_arguments};
use crate::api::request_headers::cleaning::{clean_headers, logged_headers};
use crate::api::request_headers::error::RequestHeadersError;
use crate::api::request_headers::header_metadata::add_header_metadata;
use crate::api::request_headers::provider_headers::{ProviderHeaders, provider_specific_headers};
use crate::api::request_headers::trace_context::{add_otel_trace_id, forward_traceparent};
use crate::api::request_headers::user_mapping::{internal_user_from_mapping, user_from_headers};
use crate::api::request_metadata_safety::metadata::MetadataField;
use crate::api::request_principal_metadata::PrincipalFacts;

// a header that carries a caller credential keeps it in a secret, so only the forwarded copy can read it
#[derive(Debug, Clone)]
pub enum HeaderValue {
    Plain(String),
    Credential(SecretString),
}

impl HeaderValue {
    #[must_use]
    pub fn expose(&self) -> &str {
        match self {
            Self::Plain(text) => text,
            Self::Credential(secret) => secret.expose_secret(),
        }
    }
}

pub type ForwardedHeaders = IndexMap<String, HeaderValue>;

// how the caller authenticated, which decides the credential headers that may reach the provider
#[derive(Debug, Clone, Default)]
pub struct HeaderAuth {
    pub litellm_key_header_name: Option<String>,
    pub forward_llm_provider_auth_headers: bool,
    pub authenticated_with_header: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanContext {
    pub trace_id: u128,
    pub is_valid: bool,
}

#[derive(Debug, Clone)]
pub struct RequestHeadersInput {
    pub headers: IndexMap<String, String>,
    pub data: RequestData,
    pub metadata_slot: MetadataField,
    pub auth: HeaderAuth,
    pub principal: PrincipalFacts,
    pub team_model_aliases: BTreeMap<String, String>,
    pub parent_span: Option<SpanContext>,
}

#[derive(Debug, Clone)]
pub struct RequestHeadersResult {
    pub forwarded: ForwardedHeaders,
    pub logged: IndexMap<String, String>,
    pub backend: BackendCallArguments,
    pub data: RequestData,
    pub user: Option<String>,
    pub internal_user_id: Option<String>,
    pub provider: ProviderHeaders,
}

pub struct RequestHeaders<'context> {
    pub settings: &'context Settings,
    pub model_access: &'context ModelAccessContext<'context>,
    pub limits: ApiLimits,
    // LiteLLM forwards traceparent only while an OpenTelemetry logger is configured
    pub otel_enabled: bool,
}

impl RequestHeaders<'_> {
    pub fn prepare(
        &self,
        input: RequestHeadersInput,
    ) -> Result<RequestHeadersResult, RequestHeadersError> {
        let forwarded = clean_headers(&input.headers, &input.auth);
        let mut data = input.data;
        let mut backend =
            backend_call_arguments(&forwarded, &data, &input.principal, self.settings)?;
        // the model group's forwarding list replaces the headers the general flag chose
        if let Some(group_headers) = self.model_group_headers(
            &data,
            &forwarded,
            &input.principal,
            &input.team_model_aliases,
        )? {
            backend.headers = Some(group_headers);
        }
        add_header_metadata(&forwarded, &mut data, input.metadata_slot, self.limits);
        add_otel_trace_id(&mut data, input.metadata_slot, input.parent_span);
        forward_traceparent(
            &mut data,
            &input.headers,
            self.otel_enabled && self.settings.forward_traceparent_to_llm_provider,
        );
        Ok(RequestHeadersResult {
            logged: logged_headers(&forwarded),
            user: user_from_headers(&forwarded, self.settings),
            internal_user_id: internal_user_from_mapping(
                &forwarded,
                &self.settings.user_header_mappings,
            ),
            provider: provider_specific_headers(&forwarded),
            forwarded,
            backend,
            data,
        })
    }
}
