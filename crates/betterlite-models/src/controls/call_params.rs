use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::controls::control_options::ControlOptions;
use crate::headers::affinity::{AffinityParams, SessionMetadata};

pub const TRUSTED_CREDENTIALS_KEY: &str = "_litellm_internal_model_credentials";

// Config and server code resolve these values before a call; none of the fields decode from request JSON.
#[derive(Clone, Debug, Default)]
pub struct TrustedCall {
    pub model: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub provider_affinity_header: Option<String>,
    pub control: ControlOptions,
    pub credentials: Option<TrustedCredentials>,
}

// Only server code can build the snapshot, so a request-supplied mapping can never stand in for it.
#[derive(Clone, Debug, Default)]
pub struct TrustedCredentials {
    pub values: BTreeMap<String, SecretString>,
}

// The whole request-side contribution to a call. A request that names a credential, a destination or an internal key fails to decode instead of overriding one.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestCall {
    pub session_id: Option<String>,
    pub litellm_session_id: Option<String>,
    pub metadata: Option<SessionMetadata>,
    pub litellm_metadata: Option<SessionMetadata>,
}

#[derive(Clone, Debug, Default)]
pub struct CallParams {
    pub model: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub control: Option<ControlOptions>,
    pub credentials: Option<TrustedCredentials>,
    pub affinity: AffinityParams,
}

#[must_use]
pub fn call_params(trusted: &TrustedCall, request: &RequestCall) -> CallParams {
    let mut params = CallParams {
        model: trusted.model.clone(),
        api_key: trusted.api_key.clone(),
        api_base: trusted.api_base.clone(),
        custom_llm_provider: trusted.custom_llm_provider.clone(),
        control: None,
        credentials: trusted.credentials.clone(),
        affinity: AffinityParams {
            provider_affinity_header: trusted.provider_affinity_header.clone(),
            session_id: request.session_id.clone(),
            litellm_session_id: request.litellm_session_id.clone(),
            metadata: request.metadata.clone(),
            litellm_metadata: request.litellm_metadata.clone(),
        },
    };
    with_control_options(&mut params, trusted.control);
    params
}

// A default control leaves no stored key behind, and only a validated value can be stored at all.
pub fn with_control_options(params: &mut CallParams, control: ControlOptions) {
    params.control = None;
    if control != ControlOptions::default() {
        params.control = Some(control);
    }
}

mod tests;
