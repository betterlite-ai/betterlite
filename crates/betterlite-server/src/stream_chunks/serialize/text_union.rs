use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// text is Union[ResponseTextConfigParam, dict]; its typed config drops nulls only while every declared value fits
pub(super) fn is_text_config(value: &Json) -> bool {
    let Json::Object(entries) = value else {
        return false;
    };
    format_is_config(entries.get("format")) && verbosity_is_config(entries.get("verbosity"))
}

fn format_is_config(format: Option<&Json>) -> bool {
    let Some(format) = format else {
        return true;
    };
    let Json::Object(entries) = format else {
        return matches!(format, Json::Null);
    };
    match entries.get("type") {
        Some(Json::Null) => true,
        Some(Json::Text(kind)) => match kind.as_str() {
            "text" | "json_object" => true,
            "json_schema" => schema_is_config(entries),
            _ => false,
        },
        None | Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => false,
    }
}

fn schema_is_config(entries: &JsonObject) -> bool {
    let required = entries.contains_key("name") && entries.contains_key("schema");
    let name = matches!(entries.get("name"), Some(Json::Null | Json::Text(_)));
    let schema = matches!(entries.get("schema"), Some(Json::Null | Json::Object(_)));
    let described = matches!(
        entries.get("description"),
        None | Some(Json::Null | Json::Text(_))
    );
    let strict = matches!(
        entries.get("strict"),
        None | Some(Json::Null | Json::Bool(_))
    );
    required && name && schema && described && strict
}

fn verbosity_is_config(verbosity: Option<&Json>) -> bool {
    match verbosity {
        None | Some(Json::Null) => true,
        Some(Json::Text(level)) => matches!(level.as_str(), "low" | "medium" | "high"),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => false,
    }
}
