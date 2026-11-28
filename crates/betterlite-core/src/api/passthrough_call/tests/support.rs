#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{ProviderIdentity, ResolvedModel};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use super::super::{PassthroughCallArgs, PassthroughClient};

pub(crate) const MODEL: &str = "my-model";
pub(crate) const ENDPOINT: &str = "chat/completions";
const INIT: &str = r#"{"model":"passthrough","call_type":"passthrough","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

// the injected B173 client, with the body cap the call's own client copies from it
pub(crate) fn proxy_client(body_bytes: usize) -> ProxiedClient {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    let mut client = ProxiedClient::outbound(&policy).unwrap();
    client.limits.body_bytes = body_bytes;
    client
}

// the caller-made B247 state the call updates, as the proxy and the router make it
pub(crate) fn logging_state() -> LoggingState {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let init: LoggingInit = serde_json::from_str(INIT).unwrap();
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap()
}

pub(crate) fn base_args() -> PassthroughCallArgs {
    PassthroughCallArgs {
        method: String::from("POST"),
        endpoint: String::from(ENDPOINT),
        model: String::from(MODEL),
        custom_llm_provider: Some(String::from("vllm")),
        api_base: None,
        api_key: None,
        request_query_params: None,
        request_headers: None,
        content: None,
        data: None,
        files: None,
        json: None,
        params: None,
        cookies: None,
        client: None,
        extra: Object::new(),
    }
}

pub(crate) fn vllm_args(api_base: &str) -> PassthroughCallArgs {
    let mut args = base_args();
    args.api_base = Some(api_base.to_owned());
    args
}

pub(crate) fn mantle_args(api_base: &str) -> PassthroughCallArgs {
    let mut args = base_args();
    args.model = String::from("bedrock_mantle/original");
    args.custom_llm_provider = Some(String::from("bedrock_mantle"));
    args.api_base = Some(api_base.to_owned());
    args
}

pub(crate) fn bedrock_args(api_base: &str) -> PassthroughCallArgs {
    let mut args = base_args();
    args.model = String::from("anthropic.claude");
    args.custom_llm_provider = Some(String::from("bedrock"));
    args.api_base = Some(api_base.to_owned());
    args
}

pub(crate) fn resolved(
    model: &str,
    provider: ProviderId,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> ResolvedModel {
    ResolvedModel {
        model: model.to_owned(),
        provider: ProviderIdentity::Known(provider),
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
    }
}

pub(crate) fn object(pairs: &[(&str, JsonValue)]) -> Object {
    let mut object = Object::new();
    for (name, value) in pairs {
        object.insert((*name).to_owned(), value.clone());
    }
    object
}

pub(crate) fn empty_environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub(crate) fn test_environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// the Watsonx environment the call reads when nothing but the IAM endpoint is set
pub(crate) fn watsonx_environment(iam_url: &str) -> WatsonxEnvironment {
    WatsonxEnvironment::from_lookup(&test_environment(&[("WATSONX_IAM_URL", iam_url)]))
}

pub(crate) fn client_timeouts() -> PassthroughClient {
    PassthroughClient {
        timeout: Duration::from_secs(30),
        connect_timeout: Duration::from_secs(3),
    }
}

pub(crate) fn header(reply: &super::super::PassthroughReply, name: &str) -> Option<String> {
    let found = reply
        .headers
        .iter()
        .find(|(header, _value)| header.eq_ignore_ascii_case(name));
    found.map(|(_header, value)| value.clone())
}
