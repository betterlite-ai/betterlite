#![cfg(test)]

mod cache;
mod create;
mod delete;
mod gate;
mod params;
mod rows;
mod state;
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

#[derive(Clone, Copy)]
enum Column {
    Params,
    Metadata,
}

// a row's JSONB column: Prisma stores JSON text in it, so the column is parsed for a structural assertion
fn stored_column(store: &LiteLlmManagedVectorStore, column: Column) -> Json {
    let value = match column {
        Column::Params => store.litellm_params.clone().expect("the row holds params"),
        Column::Metadata => store
            .vector_store_metadata
            .clone()
            .expect("the row holds metadata"),
    };
    let tree: Json = serde_json::from_value(value).expect("a JSON value");
    if let Json::Text(inner) = &tree {
        return serde_json::from_str(inner).expect("the column holds JSON text");
    }
    tree
}
