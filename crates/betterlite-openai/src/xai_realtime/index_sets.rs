use betterlite_models::decisions::Json;
use serde_json::Number;

const OUTPUT_INDEX_EVENTS: [&str; 12] = [
    "response.output_item.added",
    "response.output_item.done",
    "response.content_part.added",
    "response.content_part.done",
    "response.output_text.delta",
    "response.output_text.done",
    "response.output_audio_transcript.delta",
    "response.output_audio_transcript.done",
    "response.output_audio.delta",
    "response.output_audio.done",
    "response.function_call_arguments.delta",
    "response.function_call_arguments.done",
];

const CONTENT_INDEX_EVENTS: [&str; 8] = [
    "response.content_part.added",
    "response.content_part.done",
    "response.output_text.delta",
    "response.output_text.done",
    "response.output_audio_transcript.delta",
    "response.output_audio_transcript.done",
    "response.output_audio.delta",
    "response.output_audio.done",
];

#[must_use]
pub fn inject_missing_indices(event: Json, event_type: Option<&str>) -> Json {
    let Some(event_type) = event_type else {
        return event;
    };
    let needs_output = OUTPUT_INDEX_EVENTS.contains(&event_type);
    let needs_content = CONTENT_INDEX_EVENTS.contains(&event_type);
    match event {
        Json::Object(mut fields) if needs_output || needs_content => {
            if needs_output && !fields.contains_key("output_index") {
                fields.insert(
                    String::from("output_index"),
                    Json::Number(Number::from(0_u8)),
                );
            }
            if needs_content && !fields.contains_key("content_index") {
                fields.insert(
                    String::from("content_index"),
                    Json::Number(Number::from(0_u8)),
                );
            }
            Json::Object(fields)
        }
        other => other,
    }
}
