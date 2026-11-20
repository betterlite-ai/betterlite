use betterlite_models::decisions::Json;

// a text member of a JSON object: a null, missing or non-text member reads as absent
#[must_use]
pub fn text_field<'value>(object: &'value Json, name: &str) -> Option<&'value str> {
    let Json::Object(fields) = object else {
        return None;
    };
    let Some(Json::Text(text)) = fields.get(name) else {
        return None;
    };
    Some(text.as_str())
}
