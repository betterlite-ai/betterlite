use betterlite_openai::api::vector_store_records::records::VectorStoreToolParams;
use betterlite_openai::api::vector_store_records::wire::{
    VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub operation: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}

pub(crate) struct Op;

fn decode_model(kind: VectorStoreRecordKind, record: &Value) -> Result<Value, String> {
    let bytes = serde_json::to_vec(record).map_err(|error| error.to_string())?;
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    };
    let decoded =
        decode_vector_store_records(kind, &bytes, limits).map_err(|error| error.to_string())?;
    serde_json::to_value(decoded).map_err(|error| error.to_string())
}

fn management_records(input: &Input) -> Result<Value, String> {
    let records = input
        .fields
        .get("records")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("records must be an object"))?;
    let mut output = BTreeMap::new();
    for (name, kind) in [
        ("update", VectorStoreRecordKind::UpdateRequest),
        ("delete", VectorStoreRecordKind::DeleteRequest),
        ("info", VectorStoreRecordKind::InfoRequest),
        ("managed_index", VectorStoreRecordKind::ManagedIndex),
        ("index_list", VectorStoreRecordKind::IndexListResponse),
        ("index_create", VectorStoreRecordKind::IndexCreateRequest),
    ] {
        let record = records
            .get(name)
            .ok_or_else(|| format!("missing `{name}` record"))?;
        output.insert(
            name.to_owned(),
            decode_model(kind, record).map_err(|message| format!("{name}: {message}"))?,
        );
    }
    serde_json::to_value(output).map_err(|error| error.to_string())
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation.as_str() {
            "tool_params_omit_only_null" => {
                let params: VectorStoreToolParams = serde_json::from_value(json!({
                    "filters": input.fields.get("filters"),
                    "max_num_results": input.fields.get("max_num_results"),
                    "ranking_options": input.fields.get("ranking_options")
                }))
                .map_err(|error| helper_error(error.to_string()))?;
                serde_json::to_value(params.to_dict())
                    .map_err(|error| helper_error(error.to_string()))
            }
            "management_record_defaults" => management_records(input).map_err(helper_error),
            "invalid_index_list_literal" => {
                let record = input
                    .fields
                    .get("record")
                    .ok_or_else(|| helper_error(String::from("record is required")))?;
                match decode_model(VectorStoreRecordKind::IndexListResponse, record) {
                    Ok(_) => Err(validation_error(String::from(
                        "expected a literal validation error",
                    ))),
                    Err(message) => Err(validation_error(message)),
                }
            }
            "discovery_extra_body_fields" => Ok(input
                .fields
                .get("extra_body")
                .cloned()
                .unwrap_or(Value::Null)),
            _ => Ok(input.fields.get("record").cloned().unwrap_or(Value::Null)),
        }
    }
}

fn helper_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn validation_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("ValidationError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
