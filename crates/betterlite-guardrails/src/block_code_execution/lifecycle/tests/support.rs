use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;
use betterlite_guardrails::block_code_execution::initializer::{
    CallbackRegistry, InitializerError,
};
use betterlite_guardrails::block_code_execution::lifecycle::{
    BlockCodeExecutionError, BlockCodeExecutionGuardrail, BlockCodeExecutionParams,
};
use betterlite_guardrails::block_code_execution::registration::{
    ApplyGuardrailError, apply_guardrail,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink,
};

pub(crate) fn params() -> BlockCodeExecutionParams {
    BlockCodeExecutionParams::default()
}

pub(crate) fn litellm_params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).expect("valid litellm params")
}

pub(crate) fn guardrail(configured: BlockCodeExecutionParams) -> BlockCodeExecutionGuardrail {
    BlockCodeExecutionGuardrail::new(configured, true).expect("guardrail builds")
}

pub(crate) fn request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        (
            "model".to_owned(),
            GuardrailValue::Text("gpt-4o".to_owned()),
        ),
        ("messages".to_owned(), GuardrailValue::Array(Vec::new())),
    ])
}

pub(crate) fn inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: Some("gpt-4o".to_owned()),
        stream_holdback_chars: None,
    }
}

#[derive(Default)]
pub(crate) struct RecordingSink {
    records: Mutex<Vec<GuardrailInvocationRecord>>,
}

impl RecordingSink {
    pub(crate) fn records(&self) -> Vec<GuardrailInvocationRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GuardrailSpanSink for RecordingSink {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

pub(crate) fn apply(
    guardrail: &BlockCodeExecutionGuardrail,
    inputs: GuardrailApiInputs,
    data: &mut BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Result<GuardrailApiInputs, ApplyGuardrailError> {
    apply_capturing(guardrail, inputs, data, input_type).0
}

pub(crate) fn apply_capturing(
    guardrail: &BlockCodeExecutionGuardrail,
    inputs: GuardrailApiInputs,
    data: &mut BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> (
    Result<GuardrailApiInputs, ApplyGuardrailError>,
    Vec<GuardrailInvocationRecord>,
) {
    let sink = RecordingSink::default();
    let mut context = GuardrailRecordContext {
        base: &guardrail.base,
        request_data: data,
        log_target: None,
        sink: &sink,
    };
    let outcome = apply_guardrail(guardrail, inputs, &mut context, input_type);
    (outcome, sink.records())
}

pub(crate) fn constructor_error<T>(
    result: Result<T, BlockCodeExecutionError>,
) -> BlockCodeExecutionError {
    match result {
        Ok(_) => panic!("expected a constructor error"),
        Err(error) => error,
    }
}

pub(crate) fn initializer_error<T>(result: Result<T, InitializerError>) -> InitializerError {
    match result {
        Ok(_) => panic!("expected an initializer error"),
        Err(error) => error,
    }
}

pub(crate) fn guardrail_entry(name: &str, params: LitellmParams) -> Guardrail {
    Guardrail {
        guardrail_id: None,
        guardrail_name: name.to_owned(),
        litellm_params: params,
        guardrail_info: None,
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: None,
    }
}

#[derive(Default)]
pub(crate) struct FakeRegistry {
    pub(crate) callbacks: Vec<Arc<BlockCodeExecutionGuardrail>>,
}

impl CallbackRegistry for FakeRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<BlockCodeExecutionGuardrail>) {
        self.callbacks.push(guardrail);
    }
}

pub(crate) struct FailingSink;

impl GuardrailSpanSink for FailingSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Err(GuardrailSpanError::Unavailable)
    }
}
