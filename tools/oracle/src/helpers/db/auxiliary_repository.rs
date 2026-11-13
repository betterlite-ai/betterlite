use betterlite_db::auxiliary_repository::decoder::decode_managed_batch_record;
use betterlite_openai::batches::records::BatchObject;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    json_strings: Vec<JsonCall>,
    #[serde(default)]
    objects: Vec<ObjectCall>,
}

#[derive(Deserialize)]
struct JsonCall {
    name: String,
    json: String,
}

#[derive(Deserialize)]
struct ObjectCall {
    name: String,
    object: Value,
}

fn helper_error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn decoded(name: &str, stored: &[u8]) -> Value {
    match decode_managed_batch_record::<BatchObject>(stored) {
        Ok(Some(batch)) => json!({"name": name, "ok": true, "batch": batch}),
        Ok(None) | Err(_) => json!({"name": name, "ok": false}),
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut calls: Vec<Value> = Vec::new();
        for call in &input.json_strings {
            let stored = serde_json::to_vec(&call.json)
                .map_err(|source| helper_error("InvalidOutput", &source.to_string()))?;
            calls.push(decoded(&call.name, &stored));
        }
        for call in &input.objects {
            let stored = serde_json::to_vec(&call.object)
                .map_err(|source| helper_error("InvalidOutput", &source.to_string()))?;
            calls.push(decoded(&call.name, &stored));
        }
        serde_json::to_value(json!({ "calls": calls }))
            .map_err(|source| helper_error("InvalidOutput", &source.to_string()))
    }
}
