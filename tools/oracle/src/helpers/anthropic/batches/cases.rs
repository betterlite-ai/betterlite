use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_anthropic::batches::config::{AnthropicBatchesConfig, BatchesConfig, Environment};
use betterlite_anthropic::batches::decode::DecodeError;
use betterlite_anthropic::batches::error::BatchesError;
use betterlite_anthropic::batches::result::{BatchResponse, project_batch_results};
use betterlite_anthropic::batches::timestamps::Clock;
use betterlite_anthropic::response::Logging;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::header::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::{AuthCase, AuthScenario, BodiesCase, CreateCase, ErrorCase, ErrorScenario};
use super::support::{
    Silent, ZeroCounter, failure, header_map, logging_env, logging_state, plain_headers, settings,
    utc_offset,
};
use super::views::{auth_of, response_view};

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
