use betterlite_models::decisions::Json;

pub(super) fn event_type_label(event: &Json) -> String {
    let Json::Object(fields) = event else {
        return String::from("<non-string>");
    };
    match fields.get("type") {
        Some(Json::Text(kind)) => kind.clone(),
        _ => String::from("<non-string>"),
    }
}
