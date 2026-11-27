use std::collections::BTreeMap;

use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_store_http_write::service::ClientKind;

// The client a caller handed the SDK; the provider transport reuses an asynchronous one and replaces a synchronous one.
#[derive(Debug, Clone, Copy)]
pub struct SuppliedClient<'call> {
    pub kind: ClientKind,
    pub transport: &'call ProxiedClient,
}

// The declared arguments and the kwargs each SDK call reads; anything else in kwargs only feeds the pre-call log.
pub struct ClientSecretCall<'call> {
    pub model: Option<&'call str>,
    pub session: Option<&'call Json>,
    pub expires_after: Option<&'call Json>,
    pub timeout: Option<f64>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub client: Option<SuppliedClient<'call>>,
    pub kwargs: &'call IndexMap<String, Json>,
}

pub struct TranscriptionSessionCall<'call> {
    pub model: Option<&'call str>,
    pub transcription_session: Option<&'call Json>,
    pub timeout: Option<f64>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub client: Option<SuppliedClient<'call>>,
    pub kwargs: &'call IndexMap<String, Json>,
}

pub struct RealtimeCallsCall<'call> {
    pub openai_ephemeral_key: &'call str,
    pub sdp_body: &'call [u8],
    pub model: Option<&'call str>,
    pub session: Option<&'call Json>,
    pub timeout: Option<f64>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub client: Option<SuppliedClient<'call>>,
    pub kwargs: &'call IndexMap<String, Json>,
}
