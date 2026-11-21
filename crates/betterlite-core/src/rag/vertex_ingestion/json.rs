use betterlite_models::decisions::{Extra, Json};

#[must_use]
pub(super) fn object<const N: usize>(entries: [(&'static str, Json); N]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect::<Extra>(),
    )
}

#[must_use]
pub(super) fn field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub(super) fn response_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
