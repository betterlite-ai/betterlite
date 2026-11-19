use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redact::structured::redact::redact_credentials_in_payload;

// the config-load diagnostic, minus every value the shared credential walker masks
pub(crate) fn redacted_stores(stores: &[LiteLlmManagedVectorStore]) -> String {
    let records: Vec<Json> = stores.iter().map(redacted_store).collect();
    serde_json::to_string(&records).unwrap_or_else(|_| "[]".to_owned())
}

fn redacted_store(store: &LiteLlmManagedVectorStore) -> Json {
    let Ok(serialized) = serde_json::to_value(store) else {
        return Json::Null;
    };
    let Some(fields) = serialized.as_object() else {
        return Json::Null;
    };
    serde_json::from_value(redact_credentials_in_payload(fields)).unwrap_or(Json::Null)
}
