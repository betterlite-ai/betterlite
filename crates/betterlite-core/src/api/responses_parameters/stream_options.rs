use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiStreamOptions;

#[must_use]
pub fn normalize_stream_options(
    stream_options: Option<&JsonValue>,
) -> Option<ResponsesApiStreamOptions> {
    let Some(JsonValue::Object(fields)) = stream_options else {
        return None;
    };
    let Some(JsonValue::Bool(include_obfuscation)) = fields.get("include_obfuscation") else {
        return None;
    };
    Some(ResponsesApiStreamOptions {
        include_obfuscation: Some(Some(*include_obfuscation)),
    })
}
