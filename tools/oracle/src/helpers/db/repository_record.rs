pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| helper_error("InvalidInput", "operation is required"))?;
        let record = input
            .get("record")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        match operation {
            "record_to_dict" => record_to_dict(record),
            "to_model" => Ok(record),
            "to_model_list" => {
                let values = record
                    .as_array()
                    .ok_or_else(|| helper_error("ValueError", "record list is not a list"))?;
                Ok(serde_json::Value::Array(
                    values
                        .iter()
                        .filter(|value| !value.is_null())
                        .cloned()
                        .collect(),
                ))
            }
            _ => Self::not_built("repository_record operation"),
        }
    }
}

fn record_to_dict(record: serde_json::Value) -> crate::helpers::HelperResult {
    if record.is_object() {
        return Ok(record);
    }
    let pairs = record
        .as_array()
        .ok_or_else(|| helper_error("ValueError", "record is not a mapping or pair sequence"))?;
    let mut result = serde_json::Map::new();
    for pair in pairs {
        let pair_values = pair.as_array().ok_or_else(|| {
            helper_error("ValueError", "record is not a mapping or pair sequence")
        })?;
        if pair_values.len() != 2 {
            return Err(helper_error(
                "ValueError",
                "record is not a mapping or pair sequence",
            ));
        }
        let key = pair_values
            .first()
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                helper_error("ValueError", "record is not a mapping or pair sequence")
            })?;
        let value = pair_values.get(1).cloned().ok_or_else(|| {
            helper_error("ValueError", "record is not a mapping or pair sequence")
        })?;
        result.insert(key.to_owned(), value);
    }
    Ok(serde_json::Value::Object(result))
}

fn helper_error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    let error = Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    });
    if message.is_empty() {
        return error;
    }
    error
}
