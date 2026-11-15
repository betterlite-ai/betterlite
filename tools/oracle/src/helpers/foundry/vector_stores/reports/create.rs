use betterlite_foundry::vector_stores::create::{CreateStep, create_vector_store};
use serde_json::{Value, json};

use super::super::scenarios::UnsupportedStoreCreateInput;

pub(super) fn create_report(_input: &UnsupportedStoreCreateInput) -> Value {
    json!({
        "request": create_outcome(CreateStep::Request),
        "response": create_outcome(CreateStep::Response),
    })
}

fn create_outcome(step: CreateStep) -> Value {
    match create_vector_store(step) {
        Ok(never) => match never {},
        Err(_error) => json!({"ok": false, "status": Value::Null}),
    }
}
