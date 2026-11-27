use betterlite_db::budget_permission_rows::JsonData;

// A declaration is list[str]: a non-text entry grants nothing, since no allowlist check matches it.
#[must_use]
pub fn text_models(values: &[JsonData]) -> Vec<String> {
    let mut models = Vec::new();
    for value in values {
        if let JsonData::String(text) = value {
            models.push(text.clone());
        }
    }
    models
}
