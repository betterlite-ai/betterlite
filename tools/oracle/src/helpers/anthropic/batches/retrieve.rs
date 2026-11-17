use std::sync::PoisonError;

use betterlite_anthropic::batches::config::AnthropicBatchesConfig;
use betterlite_anthropic::batches::error::BatchesError;
use betterlite_anthropic::batches::handler::{
    AnthropicBatchesHandler, RetrieveRequest, default_batch_logging,
};
use betterlite_anthropic::batches::timestamps::Clock;
use betterlite_anthropic::response::Logging;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::batches::records::BatchObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{RetrieveCase, RetrieveScenario};
use super::support::{
    CLIENT_TIMEOUT, Replay, Silent, failure, logging_env, logging_state, settings, utc_offset,
};
use super::views::{constructed_view, pre_call_view, request_view};

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
