use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chat_choices::endpoint::ModelListTransport;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::proxy::images::{ImageEndpoint, image_headers, image_url};
use betterlite_openai::proxy::models::{ModelsRequest, get_models};
use betterlite_openai::proxy::responses::{PROVIDER, SUPPORTS_NATIVE_WEBSOCKET, responses_url};
use betterlite_openai::proxy::{ProxyOptions, map_params, transform_request};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::yield_now;
use url::Url;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

const DEADLINE: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Route { params: Vec<RouteCase> },
    ChatRequest { params: Vec<ChatCase> },
    ResponsesUrl { params: Vec<ResponsesCase> },
    ImageRequest { params: Vec<ImageCase> },
    Models { params: Vec<ModelsCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteCase {
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    environment_base: Option<String>,
    environment_key: Option<String>,
    environment_use_litellm_proxy: Option<bool>,
    global_use_litellm_proxy: bool,
    deployment_use_litellm_proxy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatCase {
    model: String,
    messages: Value,
    non_default: Object,
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesCase {
    api_base: Option<String>,
    environment_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageCase {
    endpoint: Endpoint,
    api_base: Option<String>,
    api_key: Option<String>,
    environment_base: Option<String>,
    environment_key: Option<String>,
    headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Endpoint {
    Generations,
    Edits,
}

impl Endpoint {
    const fn image(self) -> ImageEndpoint {
        match self {
            Self::Generations => ImageEndpoint::Generations,
            Self::Edits => ImageEndpoint::Edits,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsCase {
    api_base: Option<String>,
    api_key: Option<String>,
    environment_base: Option<String>,
    environment_key: Option<String>,
    openai_api_key: Option<String>,
    reply_status: u16,
    reply_body: String,
}

struct Script {
    status: u16,
    body: Vec<u8>,
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl Script {
    fn recording(case: &ModelsCase) -> Self {
        Self {
            status: case.reply_status,
            body: case.reply_body.clone().into_bytes(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn first_request(&self) -> Option<(String, Option<String>)> {
        let requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        requests.first().cloned()
    }
}

impl ModelListTransport for Script {
    async fn get(
        &self,
        url: &Url,
        headers: &HeaderMap,
        _deadline: Duration,
    ) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        {
            let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
            requests.push((url.to_string(), authorization));
        }
        Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        })
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Route { params } => Ok(values(params, route_value)),
            Input::ChatRequest { params } => chat_values(params),
            Input::ResponsesUrl { params } => Ok(values(params, responses_value)),
            Input::ImageRequest { params } => Ok(values(params, image_value)),
            Input::Models { params } => models_values(params),
        }
    }
}

fn values<Case>(cases: &[Case], convert: fn(&Case) -> Value) -> Value {
    let results: Vec<Value> = cases.iter().map(convert).collect();
    json!({ "results": results })
}

fn route_value(case: &RouteCase) -> Value {
    let configured = ProxyOptions {
        api_base: case.environment_base.clone(),
        api_key: case.environment_key.as_ref().map(SecretString::from),
        use_litellm_proxy: case.environment_use_litellm_proxy.unwrap_or(false)
            || case.global_use_litellm_proxy,
    };
    let explicit_key = case.api_key.as_ref().map(SecretString::from);
    let route = configured.route(&case.model, case.api_base.as_deref(), explicit_key.as_ref());
    let static_key = configured.resolve_api_key(explicit_key.as_ref());
    json!({
        "should_use": configured.should_use_litellm_proxy(case.deployment_use_litellm_proxy),
        "model": route.model,
        "provider": route.provider.to_string(),
        "api_base": route.api_base,
        "api_key": revealed(route.api_key.as_ref()),
        "get_api_key": revealed(static_key.as_ref()),
    })
}

fn revealed(key: Option<&SecretString>) -> Option<String> {
    key.map(|key| key.expose_secret().to_owned())
}

// LiteLLM's proxy config inherits OpenAIGPTConfig; the inherited list and the constants only
// need to agree on the names it accepts, which the mapping below checks
fn inherited_config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn chat_values(params: &[ChatCase]) -> HelperResult {
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let results = params
        .iter()
        .map(|case| chat_value(case, &catalog))
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(json!({ "results": results }))
}

fn chat_value(case: &ChatCase, catalog: &CatalogCapabilities) -> Result<Value, Box<HelperError>> {
    let mut optional = case.optional_params.clone();
    map_params(
        &inherited_config(),
        &case.model,
        &case.non_default,
        &mut optional,
        catalog,
    )
    .or_value_error()?;
    let request = transform_request(&case.model, case.messages.clone(), &optional);
    Ok(json!({ "mapped": optional, "request": request }))
}

fn responses_value(case: &ResponsesCase) -> Value {
    let options = ProxyOptions {
        api_base: case.environment_base.clone(),
        ..ProxyOptions::default()
    };
    match responses_url(&options, case.api_base.as_deref()) {
        Ok(url) => json!({
            "ok": true,
            "url": url,
            "provider": PROVIDER.to_string(),
            "websocket": SUPPORTS_NATIVE_WEBSOCKET,
        }),
        Err(_error) => json!({ "ok": false }),
    }
}

fn image_value(case: &ImageCase) -> Value {
    let options = ProxyOptions {
        api_base: case.environment_base.clone(),
        api_key: case.environment_key.as_ref().map(SecretString::from),
        ..ProxyOptions::default()
    };
    let explicit_key = case.api_key.as_ref().map(SecretString::from);
    match image_url(&options, case.api_base.as_deref(), case.endpoint.image()) {
        Ok(url) => json!({
            "ok": true,
            "url": url,
            "headers": image_headers(&options, &case.headers, explicit_key.as_ref()),
        }),
        Err(_error) => json!({ "ok": false }),
    }
}

fn models_values(params: &[ModelsCase]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let results: Vec<Value> = params
        .iter()
        .map(|case| models_value(&runtime, case))
        .collect();
    Ok(json!({ "results": results }))
}

fn models_value(runtime: &tokio::runtime::Runtime, case: &ModelsCase) -> Value {
    let options = ProxyOptions {
        api_base: case.environment_base.clone(),
        api_key: case.environment_key.as_ref().map(SecretString::from),
        ..ProxyOptions::default()
    };
    let explicit_key = case.api_key.as_ref().map(SecretString::from);
    let request = ModelsRequest {
        api_base: case.api_base.as_deref(),
        api_key: explicit_key.as_ref(),
        environment_api_key: case.openai_api_key.as_ref().map(SecretString::from),
        deadline: DEADLINE,
    };
    let script = Script::recording(case);
    let listed = runtime.block_on(get_models(&options, &script, &request));
    match (listed, script.first_request()) {
        (Ok(models), Some((url, authorization))) => json!({
            "ok": true,
            "models": models,
            "url": url,
            "authorization": authorization,
        }),
        _ => json!({ "ok": false }),
    }
}
