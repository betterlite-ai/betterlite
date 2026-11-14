use betterlite_db::key_repository::decode::to_model;
use betterlite_db::key_repository::record::DbRecord;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: BTreeMap<String, Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    records: Vec<Option<DbRecord>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut output = BTreeMap::new();
        for (name, case) in &input.cases {
            let mut converted = Vec::new();
            for record in &case.records {
                let value = match to_model(record.as_ref()) {
                    Ok(Some(model)) => serde_json::to_value(model).or_value_error()?,
                    Ok(None) => Value::Null,
                    Err(_invalid) => json!({"invalid": true}),
                };
                converted.push(value);
            }
            output.insert(name.clone(), converted);
        }
        serde_json::to_value(output).or_value_error()
    }
}
