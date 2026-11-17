use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::value::RawValue;

use crate::presidio_actions::request_data::RequestData;
use crate::presidio_runtime::error::PresidioRuntimeError;

// LiteLLM's hooks share one request dict, so each hook converts the guardrail data once and writes the result back once
pub(super) fn to_request_data(
    data: &BTreeMap<String, GuardrailValue>,
) -> Result<RequestData, PresidioRuntimeError> {
    data.iter()
        .map(|(key, value)| Ok((key.clone(), to_raw_value(value)?)))
        .collect()
}

pub(super) fn store_request_data(
    data: &mut BTreeMap<String, GuardrailValue>,
    request: &RequestData,
) -> Result<(), PresidioRuntimeError> {
    let mut rebuilt = BTreeMap::new();
    for (key, raw) in request {
        let value = serde_json::from_str(raw.get())
            .map_err(|source| PresidioRuntimeError::Data { source })?;
        rebuilt.insert(key.clone(), value);
    }
    *data = rebuilt;
    Ok(())
}

fn to_raw_value(value: &GuardrailValue) -> Result<Box<RawValue>, PresidioRuntimeError> {
    let encoded =
        serde_json::to_string(value).map_err(|source| PresidioRuntimeError::Data { source })?;
    RawValue::from_string(encoded).map_err(|source| PresidioRuntimeError::Data { source })
}
