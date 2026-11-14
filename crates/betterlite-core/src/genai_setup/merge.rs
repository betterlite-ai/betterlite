use betterlite_prompts::arguments::json::Object;

// _merge_native_request_fields: native fields enter through extra_body, and an explicit caller value wins on conflict
#[must_use]
pub fn merge_native_request_fields(
    native_request_fields: &Object,
    extra_body: Option<&Object>,
) -> Option<Object> {
    if native_request_fields.is_empty() && extra_body.is_none() {
        return None;
    }
    let mut merged = native_request_fields.clone();
    if let Some(caller_body) = extra_body {
        merged.extend(
            caller_body
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
    }
    Some(merged)
}
