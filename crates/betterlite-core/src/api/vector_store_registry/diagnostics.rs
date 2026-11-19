use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

const REDACTED: &str = "[REDACTED]";

// the config-load diagnostic, minus every value whose name says it carries credential material
pub(crate) fn redacted_stores(stores: &[LiteLlmManagedVectorStore]) -> String {
    let redacted: Vec<Json> = stores.iter().map(redacted_store).collect();
    serde_json::to_string(&redacted).unwrap_or_else(|_| "[]".to_owned())
}

fn redacted_store(store: &LiteLlmManagedVectorStore) -> Json {
    let mut document = store_json(store);
    redact(&mut document);
    document
}

fn store_json(store: &LiteLlmManagedVectorStore) -> Json {
    serde_json::to_value(store)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or(Json::Null)
}

fn redact(value: &mut Json) {
    match value {
        Json::Object(fields) => {
            for (name, field) in fields {
                if names_credential(name) {
                    *field = Json::Text(REDACTED.to_owned());
                } else {
                    redact(field);
                }
            }
        }
        Json::List(items) => {
            for item in items {
                redact(item);
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

fn names_credential(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    ["api_key", "secret", "password", "credential"]
        .iter()
        .any(|part| lowered.contains(*part))
}
