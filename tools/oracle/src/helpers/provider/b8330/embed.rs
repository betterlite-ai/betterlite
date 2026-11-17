use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::data_cloud_dispatch::call::EmbeddingCall;
use betterlite_provider::data_cloud_dispatch::context::{DispatchGlobals, EnvironmentSource};
use betterlite_provider::data_cloud_dispatch::embed::{EmbeddingContext, dispatch};
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixtures::{FixedClock, NoTokens, ORACLE_UPSTREAM, Scripted, environment, secrets};
use super::input::Case;
use super::wire::{error_or_null, request_view};

pub(super) fn run(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let secrets = secrets();
    let transport = Scripted::with_replies(vec![answer(case)]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = embedding_call(case)?;
    let context = EmbeddingContext {
        globals: DispatchGlobals::default(),
        env: EnvironmentSource {
            secrets: &secrets,
            environment: &env,
        },
        transport: &transport,
        tokens: &tokens,
        clock: &clock,
    };
    let outcome = runtime.block_on(dispatch(&call, case.provider.embedding_config(), &context));
    let request = recorded_request(&transport, case.provider.kept_headers())?;
    Ok(json!({"request": request, "error": error_or_null(&outcome)}))
}

fn answer(case: &Case) -> UpstreamResponse {
    UpstreamResponse {
        status: case.answer.as_ref().map_or(200, |answer| answer.status),
        headers: HeaderMap::new(),
        body: case
            .answer
            .as_ref()
            .map_or_else(String::new, |answer| answer.body.clone())
            .into_bytes(),
    }
}

fn embedding_call(case: &Case) -> Result<EmbeddingCall, Box<HelperError>> {
    Ok(EmbeddingCall {
        model: case.model.clone(),
        input: case.input.clone().unwrap_or(Json::Null),
        optional_params: case.optional_params()?,
        litellm_params: Object::new(),
        api_key: case
            .api_key
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        api_base: case
            .api_base
            .as_deref()
            .map(|value| value.replace("{{upstream}}", ORACLE_UPSTREAM)),
        headers: case.headers.clone(),
        // litellm.embedding()'s default request timeout
        timeout: Duration::from_secs(600),
    })
}

fn recorded_request(transport: &Scripted, names: &[&str]) -> Result<Value, Box<HelperError>> {
    match transport.first() {
        Some(sent) => {
            let body: Value = serde_json::from_slice(&sent.body)?;
            Ok(request_view(&sent.url, &sent.headers, names, Some(body)))
        }
        None => Ok(Value::Null),
    }
}
