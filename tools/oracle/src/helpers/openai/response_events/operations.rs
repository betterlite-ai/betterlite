use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::response_events::{
    ApiKeySources, NativeStreamingSupport, ResponsesProvider, WorkloadIdentity, get_complete_url,
    should_fake_stream, validate_environment,
};
use secrecy::SecretString;
use serde_json::Value;

use super::Probe;
use super::calls::invalid_input;
use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, thiserror::Error)]
#[error("the fixture catalog lookup failed")]
struct NativeError;

struct NativeFixture {
    answer: Option<String>,
}

impl NativeStreamingSupport for NativeFixture {
    type Error = NativeError;

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Result<Json, NativeError>> + Send {
        let outcome = match self.answer.as_deref() {
            Some("true") => Ok(Json::Bool(true)),
            Some("false") => Ok(Json::Bool(false)),
            Some(_) | None => Err(NativeError),
        };
        ready(outcome)
    }
}

pub(super) fn streams(probes: &[Probe]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| invalid_input(error.to_string()))?;
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let mut capabilities = NativeFixture {
            answer: probe.native.clone(),
        };
        let fake = runtime.block_on(should_fake_stream(
            probe.model.as_deref(),
            probe.stream,
            probe.provider.as_deref(),
            &mut capabilities,
        ));
        results.push(Value::Bool(fake));
    }
    Ok(Value::Array(results))
}

struct RecordingIdentity {
    enabled: bool,
}

impl WorkloadIdentity for RecordingIdentity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Option<SecretString> {
        if !self.enabled {
            return None;
        }
        Some(SecretString::from(format!(
            "wi:{}",
            api_base.unwrap_or("default")
        )))
    }
}

pub(super) fn auth(probes: &[Probe]) -> HelperResult {
    let results = probes
        .iter()
        .map(auth_value)
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(Value::Array(results))
}

fn auth_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let params_key = probe.api_key.as_deref().map(SecretString::from);
    let litellm_key = probe.litellm_api_key.as_deref().map(SecretString::from);
    let openai_key = probe.litellm_openai_key.as_deref().map(SecretString::from);
    let env_key = probe.env_api_key.as_deref().map(SecretString::from);
    let sources = ApiKeySources {
        litellm_params: params_key.as_ref(),
        litellm: litellm_key.as_ref(),
        openai: openai_key.as_ref(),
        environment: env_key.as_ref(),
    };
    let mut identity = RecordingIdentity {
        enabled: probe.identity.unwrap_or(false),
    };
    let given_headers: BTreeMap<String, String> = probe.headers.clone().unwrap_or_default();
    let validated = validate_environment(
        given_headers,
        sources,
        probe.api_base.as_deref(),
        ResponsesProvider::OpenAi,
        &mut identity,
    );
    serde_json::to_value(validated).map_err(|error| invalid_input(error.to_string()))
}

pub(super) fn urls(probes: &[Probe]) -> Value {
    Value::Array(
        probes
            .iter()
            .map(|probe| {
                Value::String(get_complete_url(
                    probe.api_base.as_deref(),
                    probe.litellm_api_base.as_deref(),
                    probe.env_base_url.as_deref(),
                    probe.env_api_base.as_deref(),
                ))
            })
            .collect(),
    )
}
