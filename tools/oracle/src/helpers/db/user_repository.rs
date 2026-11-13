use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use serde_json::{Map, Value};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    records: BTreeMap<String, JsonData>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut output = Map::new();
        for (name, record) in &input.records {
            let decoded = user_record_to_model(Some(record)).ok().flatten();
            let value = match decoded.as_ref().map(decoded_user) {
                Some(value) => value?,
                None => Value::Null,
            };
            output.insert(name.clone(), value);
        }
        Ok(Value::Object(output))
    }
}

fn decoded_user(user: &LiteLLMUserTable) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut serialized = serde_json::to_value(user).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    if let Value::Object(fields) = &mut serialized {
        fields.insert(
            "password_present".to_owned(),
            Value::Bool(user.password.is_some()),
        );
    }
    Ok(serialized)
}
