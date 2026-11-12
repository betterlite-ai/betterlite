use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chat_choices::endpoint::{
    ModelListRequest, ModelListTransport, complete_url, get_models, validate_environment,
};
use betterlite_openai::credentials::{ExplicitCredentials, LiteLlmGlobals, get_openai_credentials};
use betterlite_openai::openai_base_config::Headers;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{AuthSteps, CredentialCall, UrlSteps, failure};
use crate::helpers::HelperResult;

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
        Ok(names) => {
            let first = calls.first();
            json!({
                "ok": true,
                "models": names,
                "url": first.map_or(String::new(), |call| call.0.clone()),
                "authorization": first.and_then(|call| call.1.clone()),
            })
        }
        Err(_error) => json!({"ok": false}),
    }
}

pub(super) fn url_join(steps: &UrlSteps) -> Value {
    Value::Array(
        steps
            .steps
            .iter()
            .map(|step| json!({"url": complete_url(step.api_base.as_deref())}))
            .collect(),
    )
}

pub(super) fn auth(steps: &AuthSteps) -> Value {
    let mut outputs = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let key = step.api_key.clone().map(SecretString::from);
        let headers: Headers = step.headers.clone();
        outputs.push(json!(validate_environment(key.as_ref(), &headers)));
    }
    Value::Array(outputs)
}

pub(super) fn credentials(call: &CredentialCall) -> HelperResult {
    let config = ConfigLoader {
        lookup: call.env.clone(),
    }
    .load("")
    .map_err(failure)?;
    let resolved = get_openai_credentials(
        ExplicitCredentials {
            api_base: call.api_base.clone(),
            api_key: call.api_key.clone().map(SecretString::from),
            organization: None,
        },
        LiteLlmGlobals {
            api_base: call.litellm_api_base.clone(),
            api_key: call.litellm_api_key.clone().map(SecretString::from),
            openai_key: call.litellm_openai_key.clone().map(SecretString::from),
            organization: None,
        },
        &config.sections.openai_credentials,
        config.sections.resource_sdk_settings.openai_api_key.clone(),
    );
    let api_key = resolved
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned());

    Ok(json!({
        "api_key": api_key,
        "api_base": resolved.api_base,
        "base_model": call.model,
        "token_counter": "openai_api",
    }))
}
