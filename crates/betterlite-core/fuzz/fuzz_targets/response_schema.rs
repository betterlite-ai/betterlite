#![no_main]

use betterlite_core::schema::normalize::{normalize_json_schema_types, normalize_tool_schema};
use betterlite_core::schema::validate::validate_schema;
use betterlite_core::schema::{SchemaJson, SchemaLimits};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 0x0004_0000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT {
        return;
    }
    let Ok(case) = serde_json::from_slice::<SchemaJson>(bytes) else {
        return;
    };
    let Some(fields) = case.as_object() else {
        return;
    };
    let limits = SchemaLimits::default();
    if let Some(schema) = fields.get("schema") {
        drop(normalize_json_schema_types(schema, &limits));
        if let Some(response) = fields.get("response").and_then(SchemaJson::as_str) {
            drop(validate_schema(schema, response, &limits));
        }
    }
    if let Some(tool) = fields.get("tool") {
        drop(normalize_tool_schema(tool, &limits));
    }
});
