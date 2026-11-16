use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::xecguard_primitives::block_message::{grounding_block_message, scan_block_message};
use crate::xecguard_primitives::grounding_documents::extract_grounding_documents;
use crate::xecguard_primitives::history::build_full_history;
use crate::xecguard_primitives::transport::{call_grounding, call_scan};
use crate::xecguard_primitives::wire::{ScanResult, ScanType, XecGuardMessage};

use super::XecGuardGuard;
use super::construction::guardrail_name;

const BLOCKED_STATUS: u16 = 400;
const ERROR_KEY: &str = "error";
const GUARDRAIL_NAME_KEY: &str = "guardrail_name";
const RESPONSE_KEY: &str = "xecguard_response";

impl GuardrailContract for XecGuardGuard {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        scan_inputs(self, &inputs, request_data, input_type).await?;
        Ok(inputs)
    }
}

async fn scan_inputs(
    guardrail: &XecGuardGuard,
    inputs: &GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Result<(), GuardrailFailure> {
    let texts = inputs.texts.as_deref().unwrap_or_default();
    let messages = build_full_history(request_data, texts, input_type);
    if messages.is_empty() {
        return Ok(());
    }
    let scan_type = match input_type {
        GuardrailInputType::Request => ScanType::Input,
        GuardrailInputType::Response => ScanType::Response,
    };
    let Some(result) = guarded_scan(guardrail, &messages, scan_type).await? else {
        return Ok(());
    };
    if result.is_unsafe() {
        return Err(object_failure(
            guardrail,
            scan_block_message(&result),
            Some(GuardrailValue::Object(result.fields())),
        ));
    }
    match input_type {
        GuardrailInputType::Request => Ok(()),
        GuardrailInputType::Response => verify_grounding(guardrail, request_data, &messages).await,
    }
}

async fn guarded_scan(
    guardrail: &XecGuardGuard,
    messages: &[XecGuardMessage],
    scan_type: ScanType,
) -> Result<Option<ScanResult>, GuardrailFailure> {
    let called = call_scan(
        &guardrail.client,
        &guardrail.settings,
        messages,
        scan_type,
        false,
    )
    .await;
    called.map_err(|error| object_failure(guardrail, error.to_string(), None))
}

async fn verify_grounding(
    guardrail: &XecGuardGuard,
    request_data: &BTreeMap<String, GuardrailValue>,
    messages: &[XecGuardMessage],
) -> Result<(), GuardrailFailure> {
    let documents = extract_grounding_documents(request_data);
    if documents.is_empty() {
        return Ok(());
    }
    let called = call_grounding(&guardrail.client, &guardrail.settings, messages, &documents).await;
    let Some(result) =
        called.map_err(|error| object_failure(guardrail, error.to_string(), None))?
    else {
        return Ok(());
    };
    if result.is_unsafe() {
        return Err(object_failure(
            guardrail,
            grounding_block_message(&result),
            Some(GuardrailValue::Object(result.fields())),
        ));
    }
    Ok(())
}

fn object_failure(
    guardrail: &XecGuardGuard,
    message: String,
    response: Option<GuardrailValue>,
) -> GuardrailFailure {
    let mut detail = IndexMap::new();
    detail.insert(ERROR_KEY.to_owned(), GuardrailValue::Text(message));
    detail.insert(
        GUARDRAIL_NAME_KEY.to_owned(),
        GuardrailValue::Text(guardrail_name(&guardrail.base)),
    );
    if let Some(scan_response) = response {
        detail.insert(RESPONSE_KEY.to_owned(), scan_response);
    }
    // call_scan and call_grounding hand back the block_on_error text as the error's Display
    GuardrailFailure::HttpObject {
        status: BLOCKED_STATUS,
        detail,
    }
}
