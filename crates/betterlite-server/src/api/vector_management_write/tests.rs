#![cfg(test)]

mod create;
mod delete;
mod gate;
mod support;
mod update;

use axum::http::HeaderMap;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

fn at<'value>(value: &'value Json, key: &str) -> &'value Json {
    match value {
        Json::Object(fields) => fields
            .get(key)
            .unwrap_or_else(|| panic!("the object carries {key}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object carrying {key}, got {value:?}")
        }
    }
}

fn field<'value>(value: &'value Json, key: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(key),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn text(value: &Json) -> &str {
    match value {
        Json::Text(inner) => inner,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("expected text, got {value:?}")
        }
    }
}

fn field_count(value: &Json) -> usize {
    match value {
        Json::Object(fields) => fields.len(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object, got {value:?}")
        }
    }
}

fn content_type(headers: &HeaderMap) -> Option<&str> {
    headers.get("content-type")?.to_str().ok()
}

// the JSON text a stored or redacted column carries, parsed for a structural assertion
fn decoded(value: &Json) -> Json {
    serde_json::from_str(text(value)).expect("the column holds JSON text")
}

// a row's litellm_params column: Prisma stores JSON text in it
fn stored_params(store: &LiteLlmManagedVectorStore) -> Json {
    let value = store.litellm_params.clone().expect("the row holds params");
    let tree: Json = serde_json::from_value(value).expect("a JSON value");
    match tree {
        Json::Text(inner) => serde_json::from_str(&inner).expect("params text is JSON"),
        other => other,
    }
}

// a row's vector_store_metadata column: Prisma stores JSON text in it too
fn stored_metadata(store: &LiteLlmManagedVectorStore) -> Json {
    let value = store
        .vector_store_metadata
        .clone()
        .expect("the row holds metadata");
    let tree: Json = serde_json::from_value(value).expect("a JSON value");
    match tree {
        Json::Text(inner) => serde_json::from_str(&inner).expect("metadata text is JSON"),
        other => other,
    }
}
