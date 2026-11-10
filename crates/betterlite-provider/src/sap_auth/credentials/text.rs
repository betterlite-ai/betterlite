use betterlite_prompts::arguments::json::Json;

pub(super) fn lookup(document: &Json, path: &[&str]) -> Option<Json> {
    let parsed;
    let mut current = match document {
        Json::Text(text) => {
            let Ok(parsed_text) = serde_json::from_str::<Json>(text) else {
                tracing::warn!("SAP service key or VCAP service is a string but not valid JSON");
                return None;
            };
            parsed = parsed_text;
            &parsed
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => document,
    };
    for key in path {
        let Some(fields) = current.as_object() else {
            tracing::warn!("the SAP or VCAP traversal hit a non-object value at {key}");
            return None;
        };
        current = fields.get(*key)?;
    }
    Some(current.clone())
}

pub(super) fn string_value(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Bool(flag) => Some(flag.to_string()),
        Json::List(_) | Json::Object(_) => {
            tracing::warn!("a SAP AI Core credential value is a JSON array or object; skipping it");
            None
        }
    }
}
