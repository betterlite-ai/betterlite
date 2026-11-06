use std::collections::BTreeMap;
use std::fmt::Display;
use std::slice;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{LiteLlmParams, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::count_tokens::counter::{
    CountTokensCall, OpenAiTokenCounter, TokenCounter as _,
};
use betterlite_openai::count_tokens::error::CountTokensError;
use betterlite_openai::count_tokens::handler::{CountTokensHandler, HandlerRequest};
use betterlite_openai::count_tokens::messages::ChatMessage;
use betterlite_openai::count_tokens::python_text::PyValue;
use betterlite_openai::count_tokens::request::ResponsesInput;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Map, Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::{MockProvider, ProviderRequest};
use crate::routing_mock_scripts::{MockScript, redact};

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CountRequest {
    model: String,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<Map<String, Value>>>,
    system: Option<PyValue>,
    api_key: Option<String>,
    provider: Option<String>,
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default = "default_body")]
    body: Value,
    #[serde(default = "default_base_path")]
    base_path: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    // LiteLLM's module attributes have no counterpart here, so the adapter only checks that they are accepted
    #[serde(default, rename = "module_overrides")]
    _module_overrides: Option<IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrepareRequest {
    model: String,
    input: String,
    api_key: String,
    api_base: Option<String>,
    timeout: Option<u64>,
    tools: Option<Vec<Map<String, Value>>>,
    instructions: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Count(CountRequest),
    Prepare(PrepareRequest),
}

const fn default_status() -> u16 {
    200
}

fn default_body() -> Value {
    json!({"input_tokens": 7})
}

fn default_base_path() -> String {
    String::from("/v1")
}

fn fail<E>(class: &str, error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn handler() -> Result<CountTokensHandler, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|error| fail("ProxyError", &error))?;
    let client =
        ProxiedClient::outbound(&Arc::new(policy)).map_err(|error| fail("HttpError", &error))?;
    Ok(CountTokensHandler {
        client,
        request_timeout: Duration::from_mins(100),
    })
}

fn deployment(request: &CountRequest, base_url: &str) -> LiteLlmParams {
    LiteLlmParams {
        ssl_verify: None,
        model: None,
        api_base: Some(format!("{base_url}{}", request.base_path)),
        api_key: request.api_key.clone().map(SecretString::from),
        api_version: None,
        timeout: None,
        stream_timeout: None,
        max_retries: None,
        rpm: None,
        tpm: None,
        weight: None,
    }
}

fn token_counter(request: &CountRequest) -> Result<OpenAiTokenCounter, Box<HelperError>> {
    Ok(OpenAiTokenCounter {
        handler: handler()?,
        environment_api_key: request
            .env
            .get("OPENAI_API_KEY")
            .cloned()
            .map(SecretString::from),
    })
}

// The mock redacts credential headers, so the key is identified by which configured source produced it
fn key_source(seen: &ProviderRequest, request: &CountRequest) -> Option<&'static str> {
    let sent = seen.headers.get("authorization")?;
    let sources = [
        ("api_key", request.api_key.as_ref()),
        ("env", request.env.get("OPENAI_API_KEY")),
    ];
    let found = sources.into_iter().find_map(|(name, key)| {
        let mut expected = BTreeMap::new();
        expected.insert(String::from("authorization"), format!("Bearer {}", key?));
        redact(&mut expected);
        (expected.get("authorization") == Some(sent)).then_some(name)
    });
    Some(found.unwrap_or("other"))
}

async fn count(request: &CountRequest) -> HelperResult {
    let answer = UpstreamAnswer {
        status: request.status,
        headers: BTreeMap::new(),
        body: Some(request.body.clone()),
        events: None,
    };
    let provider = MockScript::try_from(slice::from_ref(&answer))
        .and_then(MockProvider::start)
        .map_err(|error| fail("MockError", &error))?;
    let host = provider.base_url.trim_start_matches("http://").to_owned();
    let params = deployment(request, &provider.base_url);
    let counter = token_counter(request)?;
    let call = CountTokensCall {
        model_to_use: &request.model,
        messages: &request.messages,
        deployment: Some(&params),
        request_model: "requested",
        tools: request.tools.as_deref(),
        system: request.system.as_ref(),
    };
    let outcome = counter.count_tokens(&call).await;
    let calls = provider
        .finish()
        .map_err(|error| fail("MockError", &error))?;
    let seen = calls.first().map(|recorded| &recorded.request);
    Ok(json!({
        "selected": counter.should_use_token_counting_api(request.provider.as_deref()),
        "result": outcome,
        "request": seen.map(|found| json!({
            "path": found.path,
            "content_type": found.headers.get("content-type"),
            "authorization_source": key_source(found, request),
            "body": found.body,
            "deployment_base": found.headers.get("host") == Some(&host),
        })),
    }))
}

fn prepare(request: &PrepareRequest) -> HelperResult {
    let key = SecretString::from(request.api_key.clone());
    let input = ResponsesInput::Text(request.input.clone());
    let call = HandlerRequest {
        model: &request.model,
        input: &input,
        api_key: &key,
        api_base: request.api_base.as_deref(),
        timeout: request.timeout.map(Duration::from_secs),
        tools: request.tools.as_deref(),
        instructions: request.instructions.as_deref(),
    };
    let prepared = handler()?
        .prepare(&call)
        .map_err(CountTokensError::into_openai_error);
    match prepared {
        Ok(outbound) => {
            let header = |name: &str| {
                outbound
                    .headers
                    .get(name)
                    .and_then(|found| found.to_str().ok())
                    .map(str::to_owned)
            };
            Ok(json!({"request": {
                "url": outbound.url.as_str(),
                "content_type": header("content-type"),
                "authorization": header("authorization"),
                "timeout": outbound.deadline.as_secs(),
            }}))
        }
        Err(error) => {
            let (status, message) = error.status_and_message();
            Ok(json!({"error": {"status": status, "message": message}}))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Count(request) => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| fail("OSError", &error))?
                .block_on(count(request)),
            Input::Prepare(request) => prepare(request),
        }
    }
}
