use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_anthropic::count_tokens::counter::{TokenCountResponse, TokenCounter as _};
use betterlite_anthropic::count_tokens::wire::CountTokensBody;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::anthropic_count::counter::{AzureAnthropicTokenCounter, CountTokensCall};
use betterlite_foundry::anthropic_count::endpoint::get_count_tokens_endpoint;
use betterlite_foundry::anthropic_count::error::ApiError;
use betterlite_foundry::anthropic_count::handler::{CountTokensRequest, FoundryCountHandler};
use betterlite_foundry::anthropic_count::reply::FoundryCountReply;
use betterlite_foundry::anthropic_messages::environment::TokenSource;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

mod fixture;
mod input;
mod sources;
mod tests;

use fixture::{
    Logs, Script, Seen, beta_version, configured_key, credential_settings, foundry_settings,
    key_shape, params_secret, request_json, request_timeout,
};
use input::{CounterCase, Globals, HandleCase, Input, Reply, failure};
use sources::Offline;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Endpoint { api_bases } => Ok(endpoint(api_bases)),
            Input::Handle { cases } => {
                let runtime = runtime()?;
                answers(cases, |case| handle_case(case, &runtime))
            }
            Input::Counter { cases } => {
                let runtime = runtime()?;
                answers(cases, |case| counter_case(case, &runtime))
            }
        }
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}

fn endpoint(api_bases: &[String]) -> Value {
    let urls: Vec<String> = api_bases
        .iter()
        .map(|base| get_count_tokens_endpoint(base))
        .collect();
    json!({ "urls": urls })
}

fn answers<Case, Answer>(cases: &[Case], answer: Answer) -> HelperResult
where
    Answer: Fn(&Case) -> Result<Value, Box<HelperError>>,
{
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        found.push(answer(case)?);
    }
    Ok(json!({ "calls": found }))
}

// The oracle never reaches Azure, so every credential source refuses
fn token_source(credentials: &DeploymentSettings) -> TokenSource<'_> {
    TokenSource {
        credentials,
        refresh_enabled: false,
        provider: None,
        sources: &Offline,
    }
}

fn scripted_handler(
    reply: Option<&Reply>,
    seconds: Option<f64>,
    globals: &Globals,
    environment: &BTreeMap<String, String>,
) -> Result<FoundryCountHandler<Script>, Box<HelperError>> {
    Ok(FoundryCountHandler {
        transport: Script::new(reply),
        request_timeout: request_timeout(seconds)?,
        beta_version: beta_version(environment)?,
        configured_key: configured_key(globals, environment)?,
    })
}

fn handle_case(case: &HandleCase, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let credentials = credential_settings(&case.litellm_params, &case.environment)?;
    let source = token_source(&credentials);
    let handler = scripted_handler(
        case.reply.as_ref(),
        case.request_timeout,
        &case.globals,
        &case.environment,
    )?;
    let key = SecretString::from(case.api_key.clone());
    let secret = params_secret(&case.litellm_params);
    let request = CountTokensRequest {
        body: CountTokensBody {
            model: &case.model,
            messages: &case.messages,
            tools: case.tools.as_ref(),
            system: case.system.as_ref(),
            optional_params: &IndexMap::new(),
        },
        api_key: &key,
        api_base: &case.api_base,
        params_key: key_shape(&case.litellm_params, secret.as_ref()),
        token: &source,
        timeout: optional_duration(case.timeout)?,
    };
    let outcome = runtime.block_on(handler.handle_count_tokens_request(&request));
    let seen = handler.transport.requests();
    Ok(json!({
        "request": seen.first().map(request_json),
        "response": response_json(outcome),
    }))
}

fn counter_call<'call>(
    case: &'call CounterCase,
    secret: &'call Option<SecretString>,
    source: &'call TokenSource<'call>,
) -> CountTokensCall<'call> {
    CountTokensCall {
        model_to_use: &case.model,
        messages: case.messages.as_ref(),
        contents: case.contents.as_ref(),
        params_key: key_shape(&case.litellm_params, secret.as_ref()),
        api_base: case
            .litellm_params
            .get("api_base")
            .and_then(|base| base.as_deref()),
        token: source,
        request_model: case.request_model.as_deref().unwrap_or(""),
        tools: case.tools.as_ref(),
        system: case.system.as_ref(),
    }
}

fn counter_case(case: &CounterCase, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let credentials = credential_settings(&case.litellm_params, &case.environment)?;
    let source = token_source(&credentials);
    let environment = foundry_settings(&case.environment);
    let counter = AzureAnthropicTokenCounter {
        handler: scripted_handler(
            case.reply.as_ref(),
            case.request_timeout,
            &case.globals,
            &case.environment,
        )?,
        environment_api_key: environment.api_key,
        environment_api_base: environment.api_base,
    };
    let secret = params_secret(&case.litellm_params);
    let call = counter_call(case, &secret, &source);
    let selected = counter.should_use_token_counting_api(case.provider.as_deref());
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let outcome = with_default(subscriber, || runtime.block_on(counter.count_tokens(&call)));
    let seen = counter.handler.transport.requests();
    Ok(counter_json(selected, outcome, &seen, &logs))
}

fn counter_json(
    selected: bool,
    outcome: Option<TokenCountResponse>,
    seen: &[Seen],
    logs: &Logs,
) -> Value {
    json!({
        "selected": selected,
        "result": outcome.map_or(Value::Null, |found| result_json(&found)),
        "request": seen.first().map(request_json),
        "warnings": logs.entries(),
    })
}

fn optional_duration(seconds: Option<f64>) -> Result<Option<Duration>, Box<HelperError>> {
    seconds
        .map(|found| Duration::try_from_secs_f64(found).map_err(failure))
        .transpose()
}

fn response_json(outcome: Result<FoundryCountReply, ApiError>) -> Value {
    match outcome {
        Ok(reply) => json!({ "value": reply.document }),
        Err(error) => json!({"error": {
            "status_code": error.status_code(),
            "processing": matches!(error, ApiError::Processing { .. }),
        }}),
    }
}

fn result_json(found: &TokenCountResponse) -> Value {
    json!({
        "total_tokens": found.total_tokens,
        "request_model": found.request_model,
        "model_used": found.model_used,
        "tokenizer_type": found.tokenizer_type,
        "original_response": found.original_response,
        "error": found.error,
        "error_message": error_message_kind(found),
        "status_code": found.status_code,
    })
}

fn error_message_kind(found: &TokenCountResponse) -> Option<String> {
    let message = found.error_message.as_deref()?;
    if found.status_code != Some(500) {
        return Some(message.to_owned());
    }
    let kind = if message.starts_with("CountTokens processing error: ") {
        "processing"
    } else {
        "generic"
    };
    Some(kind.to_owned())
}
