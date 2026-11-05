use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::METADATA_GROUNDING_KEY;
use super::wire::GroundingDocument;

#[must_use]
pub fn extract_grounding_documents(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Vec<GroundingDocument> {
    let metadata = request_data
        .get("metadata")
        .and_then(as_object)
        .filter(|fields| !fields.is_empty())
        .or_else(|| request_data.get("litellm_metadata").and_then(as_object));
    let Some(GuardrailValue::Array(documents)) =
        metadata.and_then(|fields| fields.get(METADATA_GROUNDING_KEY))
    else {
        return Vec::new();
    };
    documents.iter().filter_map(grounding_document).collect()
}

const fn as_object(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    if let GuardrailValue::Object(fields) = value {
        Some(fields)
    } else {
        None
    }
}

fn grounding_document(document: &GuardrailValue) -> Option<GroundingDocument> {
    let GuardrailValue::Object(fields) = document else {
        log_dropped_document(document);
        return None;
    };
    if let (Some(GuardrailValue::Text(document_id)), Some(GuardrailValue::Text(context))) =
        (fields.get("document_id"), fields.get("context"))
    {
        Some(GroundingDocument {
            document_id: document_id.clone(),
            context: context.clone(),
        })
    } else {
        log_dropped_document(document);
        None
    }
}

fn log_dropped_document(document: &GuardrailValue) {
    tracing::debug!(
        document = ?document,
        "XecGuard: dropping malformed grounding document"
    );
}

mod tests;
