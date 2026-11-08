use betterlite_models::decisions::Json;

use crate::langfuse_projections::content::ResponseObj;

// litellm.integrations.langfuse.langfuse_otel_attributes.get_output_content_by_type
#[must_use]
pub fn get_output_content_by_type(
    response: Option<&ResponseObj<'_>>,
    call_type: Option<&str>,
) -> String {
    let Some(present) = response else {
        return String::new();
    };
    if call_type == Some("embedding") || matches!(present, ResponseObj::Embedding) {
        return "embedding-output".to_owned();
    }
    match present {
        ResponseObj::Speech => "speech-output".to_owned(),
        ResponseObj::Dict(body) | ResponseObj::List(body) => match body {
            Json::Object(fields) if fields.is_empty() => String::new(),
            Json::List(items) if items.is_empty() => String::new(),
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::Object(_)
            | Json::List(_) => serde_json::to_string(body).unwrap_or_default(),
        },
        ResponseObj::Chat(body)
        | ResponseObj::TextCompletion(body)
        | ResponseObj::Image(body)
        | ResponseObj::Transcription(body)
        | ResponseObj::Rerank(body)
        | ResponseObj::ResponsesApi(body) => serde_json::to_string(body).unwrap_or_default(),
        ResponseObj::Embedding => String::new(),
    }
}
