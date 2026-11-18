use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::key_user_writes::hash::hash_token;
use betterlite_db::key_user_writes::json_text::{SerdeJsonEncoder, jsonify_object};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    tokens: Vec<String>,
    #[serde(default)]
    fields: BTreeMap<String, JsonData>,
}

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Function {
    ClassHash,
    ModuleHash,
    ClassJson,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::ClassHash | Function::ModuleHash => {
                let mut hashes = Vec::with_capacity(input.tokens.len());
                for token in &input.tokens {
                    hashes.push(hash_token(token));
                }
                Ok(serde_json::json!({ "hashes": hashes }))
            }
            Function::ClassJson => json_views(&input.fields),
        }
    }
}

// both sides report the encoded string's content, so json.dumps spacing never decides a case
fn json_views(fields: &BTreeMap<String, JsonData>) -> HelperResult {
    let converted = jsonify_object(fields, &SerdeJsonEncoder);
    let mut reported = BTreeMap::new();
    for (name, original) in fields {
        let entry = match (original, converted.get(name)) {
            (JsonData::Object(_), Some(JsonData::String(text))) => {
                let parsed: JsonData = serde_json::from_str(text)?;
                serde_json::json!({"kind": "encoded", "value": parsed})
            }
            (_, Some(value)) => serde_json::json!({"kind": "plain", "value": value}),
            (_, None) => serde_json::json!({"kind": "missing"}),
        };
        reported.insert(name.clone(), entry);
    }
    Ok(serde_json::json!({ "fields": reported }))
}
