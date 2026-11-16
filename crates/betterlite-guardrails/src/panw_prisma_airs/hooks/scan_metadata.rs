use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::data::Data;
use super::error::HookDataError;
use crate::panw_prisma_airs::metadata::{RequestData, ScanMetadata, prepare_metadata_from_request};

const REQUEST_KEYS: [&str; 4] = ["user", "model", "litellm_trace_id", "metadata"];

pub fn scan_metadata(data: &Data) -> Result<ScanMetadata, HookDataError> {
    let mut relevant = BTreeMap::new();
    for key in REQUEST_KEYS {
        if let Some(value) = data.get(key) {
            relevant.insert(key.to_owned(), value.clone());
        }
    }
    let encoded = serde_json::to_value(GuardrailValue::Object(relevant))
        .map_err(|source| HookDataError::Metadata { source })?;
    let request: RequestData =
        serde_json::from_value(encoded).map_err(|source| HookDataError::Metadata { source })?;
    Ok(prepare_metadata_from_request(&request))
}
