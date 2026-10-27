pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input.get("operation").and_then(serde_json::Value::as_str).ok_or_else(|| Box::new(crate::helpers::HelperError {
            class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: "operation is required".to_owned(),
        }))?;
        let record = input.get("record").cloned().unwrap_or(serde_json::Value::Null);
        match operation {
            "record_to_dict" | "to_model" => Ok(record),
            "to_model_list" => {
                let values = record.as_array().ok_or_else(|| Box::new(crate::helpers::HelperError {
                    class: "ValueError".to_owned(), status: None, error_type: None, param: None,
                    code: None, message: "record list is not a list".to_owned(),
                }))?;
                Ok(serde_json::Value::Array(values.iter().filter(|value| !value.is_null()).cloned().collect()))
            }
            _ => Self::not_built("repository_record operation"),
        }
    }
}
