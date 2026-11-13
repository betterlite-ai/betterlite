use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::crowdstrike_aidr::contract::run_guardrail;
use betterlite_guardrails::crowdstrike_aidr::error::CrowdStrikeError;
use betterlite_guardrails::crowdstrike_aidr::handler::{CrowdStrikeAidr, SUPPORTED_EVENT_HOOKS};
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::support::{
    applied_guardrails, constructor_args, dependencies, fail, input_type, outcome, projection,
    recorded_call, streaming_json,
};
use super::{ApplyInput, ConstructInput, StreamingInput};
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(super) fn apply(args: &ApplyInput) -> HelperResult {
    let (mock, base) = start_mock(args)?;
    let mut constructor = constructor_args(&args.settings);
    constructor.api_base = Some(base);
    let guardrail = CrowdStrikeAidr::new(&constructor, &args.environment, dependencies())
        .map_err(|error| fail(error.to_string()))?;
    let mut request_data = args.request_data.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| fail(error.to_string()))?;
    let result = run_guarded(&runtime, &guardrail, args, &mut request_data);
    let (path, request) = recorded_call(mock)?;
    let applied = applied_guardrails(&request_data);
    Ok(match result {
        Ok(output) => json!({
            "path": path,
            "request": request,
            "outcome": {"tag": "allow"},
            "result": projection(&output),
            "applied_guardrails": applied,
        }),
        Err(error) => json!({
            "path": path,
            "request": request,
            "outcome": outcome(&error),
            "result": Value::Null,
            "applied_guardrails": applied,
        }),
    })
}

fn start_mock(args: &ApplyInput) -> Result<(Option<MockProvider>, String), Box<HelperError>> {
    let mock = match &args.answer {
        Some(answer) => Some(
            MockScript::try_from(std::slice::from_ref(answer))
                .and_then(MockProvider::start)
                .map_err(|error| fail(error.to_string()))?,
        ),
        None => None,
    };
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => args
            .settings
            .api_base
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:1".to_owned()),
    };
    Ok((mock, base))
}

fn run_guarded(
    runtime: &Runtime,
    guardrail: &CrowdStrikeAidr,
    args: &ApplyInput,
    request_data: &mut BTreeMap<String, GuardrailValue>,
) -> Result<GuardrailApiInputs, CrowdStrikeError> {
    runtime.block_on(run_guardrail(
        guardrail,
        args.inputs.clone(),
        request_data,
        input_type(&args.input_type),
        None,
    ))
}

pub(super) fn construct(args: &ConstructInput) -> HelperResult {
    let guardrail = match CrowdStrikeAidr::new(
        &constructor_args(&args.settings),
        &args.environment,
        dependencies(),
    ) {
        Ok(guardrail) => guardrail,
        Err(CrowdStrikeError::MissingEnvironment { .. }) => {
            return Ok(json!({"tag": "missing_secret"}));
        }
        Err(error) => return Err(fail(error.to_string())),
    };
    Ok(json!({
        "api_key": guardrail.settings.api_key.expose_secret(),
        "api_base": guardrail.settings.api_base,
        "fail_on_error": guardrail.settings.fail_on_error,
        "streaming": streaming_json(&guardrail.settings.streaming),
        "event_hooks": serde_json::to_value(SUPPORTED_EVENT_HOOKS).unwrap_or(Value::Null),
    }))
}

pub(super) fn streaming_params(args: &StreamingInput) -> HelperResult {
    let constructor = constructor_args(&args.settings);
    let mut guardrail = CrowdStrikeAidr::new(&constructor, &args.environment, dependencies())
        .map_err(|error| fail(error.to_string()))?;
    let params: LitellmParams =
        serde_json::from_value(args.params.clone()).map_err(|error| fail(error.to_string()))?;
    guardrail
        .update_in_memory_params(&params)
        .map_err(|error| fail(error.to_string()))?;
    Ok(streaming_json(&guardrail.settings.streaming))
}
