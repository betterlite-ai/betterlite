use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::controls::control_options::ControlOptions;
use crate::controls::optional_params::OptionalParams;
use crate::headers::affinity::{AffinityParams, SessionMetadata};

pub const TRUSTED_CREDENTIALS_KEY: &str = "_litellm_internal_model_credentials";

// The declared type of the drop_params knob: a flag, or text LiteLLM reads with its flag parser.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum DropParams {
    Flag(bool),
    Text(String),
}

// Config and server code resolve these values before a call; none of the fields decode from request JSON.
#[derive(Clone, Debug, Default)]
pub struct TrustedCall {
    pub model: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub control: ControlOptions,
    pub credentials: Option<TrustedCredentials>,
    pub drop_params: Option<DropParams>,
    pub no_log: Option<bool>,
    pub optional: OptionalParams,
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
    pub litellm_trace_id: Option<String>,
    pub metadata: Option<SessionMetadata>,
    pub litellm_metadata: Option<SessionMetadata>,
}

#[derive(Clone, Debug, Default)]
pub struct CallParams {
    pub model: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub metadata: Option<SessionMetadata>,
    pub litellm_metadata: Option<SessionMetadata>,
    pub litellm_session_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub no_log: Option<bool>,
    pub drop_params: Option<bool>,
    pub control: Option<ControlOptions>,
    pub credentials: Option<TrustedCredentials>,
    pub optional: OptionalParams,
    pub affinity: AffinityParams,
}

#[must_use]
pub fn call_params(trusted: &TrustedCall, request: &RequestCall) -> CallParams {
    let resolved = resolved_metadata(request);
    let (litellm_session_id, litellm_trace_id) = derived_session_ids(request, resolved.as_ref());
    let mut params = CallParams {
        model: trusted.model.clone(),
        api_key: trusted.api_key.clone(),
        api_base: trusted.api_base.clone(),
        custom_llm_provider: trusted.custom_llm_provider.clone(),
        metadata: resolved,
        litellm_metadata: request.litellm_metadata.clone(),
        litellm_session_id,
        litellm_trace_id,
        no_log: trusted.no_log,
        drop_params: normalize_drop_params(trusted.drop_params.as_ref()),
        control: None,
        credentials: trusted.credentials.clone(),
        optional: trusted.optional.clone(),
        affinity: AffinityParams {
            provider_affinity_header: trusted.optional.provider_affinity_header.clone(),
            session_id: request.session_id.clone(),
            litellm_session_id: request.litellm_session_id.clone(),
            metadata: request.metadata.clone(),
            litellm_metadata: request.litellm_metadata.clone(),
        },
    };
    with_control_options(&mut params, trusted.control);
    params
}

// LiteLLM derives the call's session and trace ids from the resolved metadata when the caller passed neither.
fn derived_session_ids(
    request: &RequestCall,
    metadata: Option<&SessionMetadata>,
) -> (Option<String>, Option<String>) {
    let session_id = request.litellm_session_id.clone().or_else(|| {
        let resolved = metadata?;
        resolved
            .session_id
            .clone()
            .or_else(|| resolved.trace_id.clone())
    });
    let trace_id = request.litellm_trace_id.clone().or_else(|| {
        let resolved = metadata?;
        resolved
            .trace_id
            .clone()
            .or_else(|| resolved.session_id.clone())
    });
    (session_id, trace_id)
}

// LiteLLM resolves absent or empty metadata to a copy of litellm_metadata before it derives the call's session and trace ids.
fn resolved_metadata(request: &RequestCall) -> Option<SessionMetadata> {
    match (&request.metadata, &request.litellm_metadata) {
        (Some(metadata), Some(litellm_metadata)) if metadata.is_empty() => {
            Some(litellm_metadata.clone())
        }
        (None, Some(litellm_metadata)) => Some(litellm_metadata.clone()),
        (metadata, _) => metadata.clone(),
    }
}

// LiteLLM reads a drop_params string with its flag parser and leaves a value it cannot read unset.
#[must_use]
pub fn normalize_drop_params(drop_params: Option<&DropParams>) -> Option<bool> {
    match drop_params? {
        DropParams::Flag(flag) => Some(*flag),
        DropParams::Text(text) => betterlite_env::flag("drop_params", text).ok().flatten(),
    }
}

// A default control leaves no stored key behind, and only a validated value can be stored at all.
pub fn with_control_options(params: &mut CallParams, control: ControlOptions) {
    params.control = None;
    if control != ControlOptions::default() {
        params.control = Some(control);
    }
}

mod tests;
