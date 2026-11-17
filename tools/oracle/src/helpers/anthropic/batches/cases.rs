use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::PoisonError;

use betterlite_anthropic::batches::config::{AnthropicBatchesConfig, BatchesConfig, Environment};
use betterlite_anthropic::batches::decode::DecodeError;
use betterlite_anthropic::batches::error::BatchesError;
use betterlite_anthropic::batches::handler::{
    AnthropicBatchesHandler, RetrieveRequest, default_batch_logging,
};
use betterlite_anthropic::batches::result::{BatchResponse, project_batch_results};
use betterlite_anthropic::batches::timestamps::Clock;
use betterlite_anthropic::response::Logging;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::batches::records::BatchObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::FixedOffset;
use http::header::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{
    AuthCase, AuthScenario, BodiesCase, CreateCase, ErrorCase, ErrorScenario, RetrieveCase,
    RetrieveScenario,
};
use super::support::{
    CLIENT_TIMEOUT, Replay, Silent, ZeroCounter, failure, header_map, logging_env, logging_state,
    plain_headers, settings,
};
use super::views::{auth_of, constructed_view, pre_call_view, request_view, response_view};

pub(crate) fn create_requests(case: &CreateCase) -> HelperResult {
    let config = AnthropicBatchesConfig {
        settings: settings(&case.environment)?,
    };
    let mut urls = Vec::new();
    for base in &case.url_bases {
        urls.push(config.get_complete_batch_url(base.as_deref()));
    }
    Ok(json!({
        "urls": urls,
        "create_request": create_raised(config.transform_create_batch_request()),
        "create_response": create_raised(config.transform_create_batch_response()),
    }))
}

fn create_raised<T, E>(result: Result<T, E>) -> Value
where
    E: Display,
{
    match result {
        Err(error) => json!({"class": "NotImplementedError", "message": error.to_string()}),
        Ok(_) => Value::Null,
    }
}

pub(crate) fn lifecycle(case: &BodiesCase) -> HelperResult {
    let config = AnthropicBatchesConfig {
        settings: settings(&BTreeMap::new())?,
    };
    let clock = Clock {
        now: case.now,
        zone: utc_offset()?,
    };
    let mut values = Vec::new();
    for body in &case.bodies {
        values.push(decoded_or_raised(&config, body, &clock)?);
    }
    Ok(json!({"batches": values}))
}

fn utc_offset() -> Result<FixedOffset, Box<HelperError>> {
    let Some(offset) = FixedOffset::east_opt(0) else {
        return Err(failure("the UTC offset is not representable"));
    };
    Ok(offset)
}

fn decoded_or_raised(
    config: &AnthropicBatchesConfig,
    body: &str,
    clock: &Clock,
) -> Result<Value, Box<HelperError>> {
    match config.transform_retrieve_batch_response(body.as_bytes(), clock) {
        Ok(batch) => Ok(json!({"batch": serde_json::to_value(&batch).map_err(failure)?})),
        Err(_error) => Ok(json!({"raised": true})),
    }
}

pub(crate) fn error_counts(case: &ErrorCase) -> HelperResult {
    let config = AnthropicBatchesConfig {
        settings: settings(&BTreeMap::new())?,
    };
    let clock = Clock {
        now: case.now,
        zone: utc_offset()?,
    };
    let mut bodies = Vec::new();
    for body in &case.bodies {
        bodies.push(parse_outcome(&config, body, &clock)?);
    }
    let mut errors = Vec::new();
    for scenario in &case.errors {
        errors.push(error_record(scenario)?);
    }
    Ok(json!({"bodies": bodies, "errors": errors}))
}

fn parse_outcome(
    config: &AnthropicBatchesConfig,
    body: &str,
    clock: &Clock,
) -> Result<Value, Box<HelperError>> {
    match config.transform_retrieve_batch_response(body.as_bytes(), clock) {
        Ok(batch) => Ok(json!({"batch": serde_json::to_value(&batch).map_err(failure)?})),
        Err(DecodeError::Parse { .. }) => Ok(json!({"prefix": true})),
        Err(DecodeError::Shape { .. } | DecodeError::CountOverflow) => Ok(json!({"prefix": false})),
    }
}

fn error_record(scenario: &ErrorScenario) -> Result<Value, Box<HelperError>> {
    let pairs = scenario
        .headers
        .as_deref()
        .or(scenario.headers_object.as_deref());
    let headers = pairs.map(header_map).transpose()?;
    let error = BatchesError::provider(&scenario.message, scenario.status, headers.as_ref());
    let BatchesError::Provider(provider) = error else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "status": provider.status_code,
        "message": provider.message.as_str(),
        "headers": pairs_value(provider.headers.as_deref()),
    }))
}

fn pairs_value(pairs: Option<&[(String, String)]>) -> Value {
    match pairs {
        Some(pairs) => json!(pairs.iter().cloned().collect::<BTreeMap<String, String>>()),
        None => Value::Null,
    }
}

pub(crate) fn retrieve(case: &RetrieveCase) -> HelperResult {
    let mut scenarios = Vec::new();
    for scenario in &case.scenarios {
        scenarios.push(retrieve_scenario(case.now, scenario)?);
    }
    Ok(json!({"scenarios": scenarios}))
}

fn retrieve_scenario(now: i64, scenario: &RetrieveScenario) -> Result<Value, Box<HelperError>> {
    let recorder = Replay::new(scenario.status, &scenario.headers, &scenario.body)?;
    let handler = handler_for(now, scenario, &recorder)?;
    let context = default_batch_logging(&scenario.batch_id);
    let constructed = constructed_view(&context);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let silent = Silent;
    let mut state = logging_state(&scenario.batch_id, now, &redactor)?;
    let env = logging_env(now, &redactor, &silent)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    let result = send(
        &runtime,
        &handler,
        scenario,
        Logging {
            state: &mut state,
            env: &env,
        },
    );
    let (batch, raised) = outcome(&result)?;
    let logging = logging_value(&result, &constructed, &state);
    Ok(json!({
        "sync": scenario.sync,
        "requests": sent_requests(&recorder),
        "batch": batch,
        "raised": raised,
        "logging": logging,
    }))
}

fn handler_for<'call>(
    now: i64,
    scenario: &RetrieveScenario,
    recorder: &'call Replay,
) -> Result<AnthropicBatchesHandler<&'call Replay>, Box<HelperError>> {
    Ok(AnthropicBatchesHandler {
        transport: recorder,
        config: AnthropicBatchesConfig {
            settings: settings(&scenario.environment)?,
        },
        clock: Clock {
            now,
            zone: utc_offset()?,
        },
        request_timeout: CLIENT_TIMEOUT,
    })
}

fn send(
    runtime: &Runtime,
    handler: &AnthropicBatchesHandler<&Replay>,
    scenario: &RetrieveScenario,
    logging: Logging<'_>,
) -> Result<BatchObject, BatchesError> {
    // the source accepts the call timeout and retry count but never forwards them to the client
    let api_key = scenario.api_key.as_deref().map(SecretString::from);
    let request = RetrieveRequest {
        batch_id: &scenario.batch_id,
        api_base: scenario.api_base.as_deref(),
        api_key: api_key.as_ref(),
        timeout: None,
        max_retries: None,
    };
    runtime.block_on(handler.retrieve_batch(&request, logging))
}

fn outcome(result: &Result<BatchObject, BatchesError>) -> Result<(Value, Value), Box<HelperError>> {
    match result {
        Ok(batch) => Ok((serde_json::to_value(batch).map_err(failure)?, Value::Null)),
        Err(BatchesError::Provider(provider)) => Ok((
            Value::Null,
            json!({
                "kind": "status",
                "status": provider.status_code,
                "message": provider.message.as_str(),
            }),
        )),
        Err(error) => Ok((
            Value::Null,
            json!({"kind": "ValueError", "message": error.to_string()}),
        )),
    }
}

// the handler builds its logging object right after the key check, so a missing key never reached pre_call
fn logging_value(
    result: &Result<BatchObject, BatchesError>,
    constructed: &Value,
    state: &LoggingState,
) -> Value {
    if matches!(result, Err(BatchesError::MissingApiKey)) {
        return Value::Null;
    }
    json!({
        "constructed": constructed,
        "pre_call": pre_call_view(&state.model_call_details),
    })
}

fn sent_requests(recorder: &Replay) -> Vec<Value> {
    let seen = std::mem::take(&mut *recorder.seen.lock().unwrap_or_else(PoisonError::into_inner));
    seen.iter().map(request_view).collect()
}

pub(crate) fn result_response(case: &BodiesCase) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let silent = Silent;
    let mut state = logging_state("batch_result", case.now, &redactor)?;
    let env = logging_env(case.now, &redactor, &silent)?;
    let headers = HeaderMap::new();
    let mut values = Vec::new();
    for body in &case.bodies {
        let mut envelope = ResponseEnvelope::new(String::new());
        let response = BatchResponse {
            status: 200,
            headers: &headers,
            body: body.as_bytes(),
            now: case.now,
        };
        let projected = project_batch_results(
            &mut envelope,
            response,
            Logging {
                state: &mut state,
                env: &env,
            },
            &ZeroCounter,
        );
        match projected {
            Ok(()) => values.push(json!({"response": response_view(&envelope)})),
            Err(_error) => values.push(json!({"raised": true})),
        }
    }
    Ok(json!({"bodies": values}))
}

pub(crate) fn auth_headers(case: &AuthCase) -> HelperResult {
    let mut scenarios = Vec::new();
    for scenario in &case.scenarios {
        scenarios.push(auth_scenario(scenario)?);
    }
    Ok(json!({"scenarios": scenarios}))
}

fn auth_scenario(scenario: &AuthScenario) -> Result<Value, Box<HelperError>> {
    let config = AnthropicBatchesConfig {
        settings: settings(&scenario.environment)?,
    };
    let mut headers = header_map(&scenario.caller_headers)?;
    let api_key = scenario.api_key.as_deref().map(SecretString::from);
    let environment = Environment {
        api_key: api_key.as_ref(),
        api_base: scenario.api_base.as_deref(),
        params_api_base: scenario.params_api_base.as_deref(),
    };
    if let Err(error) = config.validate_environment(&mut headers, environment) {
        return Ok(json!({
            "headers": Value::Null,
            "auth": Value::Null,
            "raised": {"kind": "ValueError", "message": error.to_string()},
        }));
    }
    Ok(json!({
        "headers": plain_headers(&headers),
        "auth": auth_of(&headers, scenario.expected_auth.as_deref()),
        "raised": Value::Null,
    }))
}
