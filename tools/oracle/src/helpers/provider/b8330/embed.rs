use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::data_cloud_dispatch::call::EmbeddingCall;
use betterlite_provider::data_cloud_dispatch::context::DispatchGlobals;
use betterlite_provider::data_cloud_dispatch::embed::{EmbeddingContext, dispatch};
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixture::{FixedClock, NoTokens, ORACLE_UPSTREAM, Scripted, environment, secrets};
use super::input::Case;
use super::wire::{body_request_view, error_or_null, reply_view};

pub(super) fn run(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let secrets = secrets();
    let answer = UpstreamResponse {
        status: case.answer_status(),
        headers: HeaderMap::new(),
        body: case.answer_body().into_bytes(),
    };
    let transport = Scripted::with_replies(vec![answer]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = EmbeddingCall {
        model: case.model.clone(),
        input: match &case.input {
            Some(input) => input.clone(),
            None => Json::Null,
        },
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
    };
    let context = EmbeddingContext {
        globals: DispatchGlobals::default(),
        secrets: &secrets,
        environment: &env,
        transport: &transport,
        tokens: &tokens,
        clock: &clock,
    };
    let outcome = runtime.block_on(dispatch(&call, case.provider.embedding_config(), &context));
    let request = match transport.first() {
        Some(sent) => body_request_view(&sent, case.provider.kept_headers())?,
        None => Value::Null,
    };
    let (reply, error) = match &outcome {
        Ok(reply) => (reply_view(reply), Value::Null),
        Err(_) => (Value::Null, error_or_null(&outcome)),
    };
    Ok(json!({"request": request, "reply": reply, "error": error}))
}
