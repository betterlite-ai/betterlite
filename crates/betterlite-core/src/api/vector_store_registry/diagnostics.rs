use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redact::structured::redact::redact_credentials_in_payload;

// one registry record, minus every value the shared credential walker masks
pub(crate) fn redacted_store(store: &LiteLlmManagedVectorStore) -> Json {
    let Ok(serialized) = serde_json::to_value(store) else {
        return Json::Null;
    };
    let Some(fields) = serialized.as_object() else {
        return Json::Null;
    };
    serde_json::from_value(redact_credentials_in_payload(fields)).unwrap_or(Json::Null)
}
