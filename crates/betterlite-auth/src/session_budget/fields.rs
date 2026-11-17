use betterlite_models::decisions::Json;

// a text member of a JSON object: a null, missing or non-text member reads as absent
pub(crate) fn text_field<'input>(object: &'input Json, name: &str) -> Option<&'input str> {
    let Json::Object(fields) = object else {
        return None;
    };
    let Some(Json::Text(text)) = fields.get(name) else {
        return None;
    };
    Some(text.as_str())
}
