use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::invocation::{
    GuardrailHookName, GuardrailInvocation,
};
use betterlite_guardrails::guardrail_records::metadata::add_guardrail_record;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

type Fields = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Deserialize)]
pub(crate) struct RecordArgs {
    guardrail_name: Option<String>,
    event_hook: Option<LitellmParamsMode>,
    event_type: Option<GuardrailEventHooks>,
    guardrail_response: GuardrailValue,
    guardrail_status: GuardrailStatus,
    start_time: Option<f64>,
    end_time: Option<f64>,
    duration: Option<f64>,
    masked_entity_count: Option<BTreeMap<String, i64>>,
    guardrail_provider: Option<String>,
    request_data: Fields,
    litellm_params: Option<Fields>,
    model_call_params: Option<Fields>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Hook {
    PreCall,
    PostCall,
    ApplyGuardrail,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SummarizeArgs {
    hook: Hook,
    request_data: Fields,
    inputs: GuardrailApiInputs,
    response: Option<GuardrailValue>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Record(Box<RecordArgs>),
    Summarize(Box<SummarizeArgs>),
}

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn base_named(guardrail_name: Option<String>) -> GuardrailBase {
    let mut base = GuardrailBase {
        settings: GuardrailSettings::default(),
        mode: None,
    };
    base.settings.guardrail_name = guardrail_name;
    base
}

fn record(args: &RecordArgs) -> HelperResult {
    let mut base = base_named(args.guardrail_name.clone());
    base.settings.event_hook.clone_from(&args.event_hook);
    let mut request_data = args.request_data.clone();
    let mut target = GuardrailLogTarget {
        litellm_params: args.litellm_params.clone(),
        model_call_params: args.model_call_params.clone(),
    };
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data: &mut request_data,
        log_target: Some(&mut target),
        sink: &DiscardSink,
    };
    let input = GuardrailRecordInput {
        guardrail_response: args.guardrail_response.clone(),
        guardrail_status: args.guardrail_status,
        start_time: args.start_time,
        end_time: args.end_time,
        duration: args.duration,
        masked_entity_count: args.masked_entity_count.clone(),
        guardrail_provider: args.guardrail_provider.clone(),
        event_type: args.event_type,
        tracing_detail: None,
    };
    add_guardrail_record(&mut context, input).or_value_error()?;
    Ok(json!({
        "request_data": serde_json::to_value(&request_data).or_value_error()?,
        "litellm_params": serde_json::to_value(&target.litellm_params).or_value_error()?,
        "model_call_params": serde_json::to_value(&target.model_call_params).or_value_error()?,
    }))
}

fn summarize(args: &SummarizeArgs) -> HelperResult {
    let base = base_named(Some("probe".to_owned()));
    let hook = match args.hook {
        Hook::PreCall => GuardrailHookName::PreCall,
        Hook::PostCall => GuardrailHookName::PostCallSuccess,
        Hook::ApplyGuardrail => GuardrailHookName::ApplyGuardrail,
    };
    let started_at = UNIX_EPOCH
        .checked_add(Duration::from_secs(1))
        .unwrap_or(UNIX_EPOCH);
    let invocation =
        GuardrailInvocation::begin(hook, &args.request_data, Some(&args.inputs), started_at)
            .or_value_error()?;
    let mut request_data = args.request_data.clone();
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data: &mut request_data,
        log_target: None,
        sink: &DiscardSink,
    };
    invocation
        .finish_success(&mut context, args.response.as_ref(), started_at)
        .or_value_error()?;
    Ok(json!({ "request_data": serde_json::to_value(&request_data).or_value_error()? }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Record(args) => record(args),
            Input::Summarize(args) => summarize(args),
        }
    }
}
