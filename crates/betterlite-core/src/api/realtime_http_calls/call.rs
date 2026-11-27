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

// The arguments the three SDK methods share; anything else in kwargs only feeds the pre-call log.
#[derive(Clone)]
pub struct CallCommon<'call> {
    pub model: Option<&'call str>,
    pub timeout: Option<f64>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub client: Option<SuppliedClient<'call>>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// acreate_realtime_client_secret's declared arguments.
pub struct ClientSecretCall<'call> {
    pub common: CallCommon<'call>,
    pub session: Option<&'call Json>,
    pub expires_after: Option<&'call Json>,
}

// acreate_realtime_transcription_session's declared arguments.
pub struct TranscriptionSessionCall<'call> {
    pub common: CallCommon<'call>,
    pub transcription_session: Option<&'call Json>,
}

// arealtime_calls' declared arguments.
pub struct RealtimeCallsCall<'call> {
    pub common: CallCommon<'call>,
    pub openai_ephemeral_key: &'call str,
    pub sdp_body: &'call [u8],
    pub session: Option<&'call Json>,
}
