use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chat_choices::config::{
    ChatCompletionConfig as _, CredentialInputs, OpenAiChatConfig, resolve_credentials,
};
use betterlite_openai::chat_choices::endpoint::{
    ModelListRequest, ModelListTransport, complete_url, get_models, validate_environment,
};
use betterlite_openai::count_tokens::counter::OpenAiTokenCounter;
use betterlite_openai::count_tokens::handler::CountTokensHandler;
use betterlite_openai::openai_base_config::Headers;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, to_value};
use url::Url;

use super::{AuthSteps, CredentialCall, UrlSteps};
use crate::helpers::{HelperError, HelperResult};

const DEADLINE: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelListSteps {
    steps: Vec<ModelListStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelListStep {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    reply: ScriptReply,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptReply {
    status: u16,
    body: String,
}

struct Fixture {
    status: u16,
    body: String,
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl ModelListTransport for Fixture {
    fn get(
        &self,
        url: &Url,
        headers: &HeaderMap,
        _deadline: Duration,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_string(), authorization));
        let body = self.body.clone().into_bytes();
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body,
        }))
    }
}

pub(super) fn model_list(steps: &ModelListSteps) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(failure)?;
    let mut outputs = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let fixture = Fixture {
            status: step.reply.status,
            body: step.reply.body.clone(),
            requests: Mutex::new(Vec::new()),
        };
        let request = ModelListRequest {
            api_key: step.api_key.clone().map(SecretString::from),
            api_base: step.api_base.as_deref(),
            environment_api_key: step
                .env
                .get("OPENAI_API_KEY")
                .map(|key| SecretString::from(key.clone())),
            deadline: DEADLINE,
        };
        let models = runtime.block_on(get_models(&fixture, &request));
        let calls = fixture
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        outputs.push(outcome(models, &calls));
    }
    Ok(Value::Array(outputs))
}

fn outcome(
    models: Result<Vec<String>, betterlite_openai::chat_choices::endpoint::ModelListError>,
    calls: &[(String, Option<String>)],
) -> Value {
    match models {
        Ok(names) => value(&[
            ("ok", Value::Bool(true)),
            ("models", Value::Array(names.into_iter().map(Value::String).collect())),
            ("url", Value::String(calls.first().map_or_else(String::new, |call| call.0.clone()))),
            (
                "authorization",
                calls
                    .first()
                    .and_then(|call| call.1.clone())
                    .map_or(Value::Null, Value::String),
            ),
        ]),
        Err(_error) => value(&[("ok", Value::Bool(false))]),
    }
}

pub(super) fn url_join(steps: &UrlSteps) -> Value {
    Value::Array(
        steps
            .steps
            .iter()
            .map(|step| value(&[("url", Value::String(complete_url(step.api_base.as_deref())))]))
            .collect(),
    )
}

pub(super) fn auth(steps: &AuthSteps) -> Value {
    Value::Array(
        steps
            .steps
            .iter()
            .map(|step| {
                let key = step.api_key.clone().map(SecretString::from);
                let headers: Headers = step.headers.clone();
                to_value(validate_environment(key.as_ref(), &headers)).unwrap_or(Value::Null)
            })
            .collect(),
    )
}

pub(super) fn credentials(call: &CredentialCall) -> HelperResult {
    let config = ConfigLoader {
        lookup: call.env.clone(),
    }
    .load("")
    .map_err(failure)?;
    let settings = &config.sections.openai_credentials;
    let inputs = CredentialInputs {
        api_key: call.api_key.clone().map(SecretString::from),
        litellm_api_key: call.litellm_api_key.clone().map(SecretString::from),
        litellm_openai_key: call.litellm_openai_key.clone().map(SecretString::from),
        environment_api_key: config.sections.resource_sdk_settings.openai_api_key.clone(),
        api_base: call.api_base.as_deref(),
        litellm_api_base: call.litellm_api_base.as_deref(),
        settings,
    };
    let resolved = resolve_credentials(inputs);
    let counter: OpenAiTokenCounter = OpenAiChatConfig.token_counter(handler()?, None);
    let base_model = OpenAiChatConfig.base_model(Some(call.model.as_str()));
    drop(counter);

    Ok(value(&[
        (
            "api_key",
            resolved
                .api_key
                .as_ref()
                .map_or(Value::Null, |key| Value::String(key.expose_secret().to_owned())),
        ),
        ("api_base", Value::String(resolved.api_base)),
        ("base_model", base_model.map_or(Value::Null, Value::String)),
        ("token_counter", Value::String(String::from("openai_api"))),
    ]))
}

fn handler() -> Result<CountTokensHandler, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).map_err(failure)?,
    );
    let client =
        betterlite_http::proxy::ProxiedClient::outbound(&policy).map_err(failure)?;
    Ok(CountTokensHandler {
        client,
        request_timeout: DEADLINE,
    })
}

pub(super) fn value(fields: &[(&str, Value)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(name, field)| ((*name).to_owned(), field.clone()))
            .collect(),
    )
}

fn failure(error: impl std::fmt::Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
