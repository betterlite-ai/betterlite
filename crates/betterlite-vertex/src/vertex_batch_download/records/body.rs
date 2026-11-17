use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails, Usage};
use http::header::HeaderMap;
use indexmap::IndexMap;
use serde_json::Number;

use crate::gemini_candidates::choice::{Choice, Choices, Message};
use crate::gemini_response::complete::{
    CompleteRequest, CompleteResponse, assemble_complete_response,
};
use crate::gemini_response::decode::decode_generate_content;
use crate::gemini_response_parts::projection::CallMode;
use crate::vertex_batch_download::records::ConversionContext;
use crate::vertex_batch_download::records::error::ConversionError;

// pydantic's exclude_unset drops a message field the candidates never set, and keeps null in the four it always sets
const CONDITIONAL_MESSAGE_FIELDS: [&str; 6] = [
    "reasoning_content",
    "thinking_blocks",
    "images",
    "audio",
    "annotations",
    "provider_specific_fields",
];

pub(crate) fn completion_body(
    response: &IndexMap<String, Json>,
    model: &str,
    context: &mut ConversionContext,
) -> Result<IndexMap<String, Json>, ConversionError> {
    let encoded = serde_json::to_vec(response).map_err(ConversionError::Encode)?;
    let decoded = decode_generate_content(&encoded)?;
    let complete = assemble_complete_response(
        &decoded,
        &HeaderMap::new(),
        &CompleteRequest {
            model,
            now: context.now,
            call_mode: CallMode::Tools,
        },
        context.ids.as_mut(),
    )?;
    project(&complete, context.now)
}

fn project(
    response: &CompleteResponse,
    now: i64,
) -> Result<IndexMap<String, Json>, ConversionError> {
    let mut body = IndexMap::new();
    body.insert(String::from("id"), Json::Text(response.id.clone()));
    body.insert(String::from("created"), Json::Number(Number::from(now)));
    body.insert(String::from("model"), Json::Text(response.model.clone()));
    body.insert(
        String::from("object"),
        Json::Text(String::from("chat.completion")),
    );
    body.insert(String::from("system_fingerprint"), Json::Null);
    body.insert(String::from("choices"), choices_body(&response.choices)?);
    body.insert(
        String::from("usage"),
        usage_body(&response.usage, response.metadata.is_none()),
    );
    if let Some(metadata) = &response.metadata {
        for (name, values) in [
            ("vertex_ai_grounding_metadata", &metadata.grounding_metadata),
            (
                "vertex_ai_url_context_metadata",
                &metadata.url_context_metadata,
            ),
            ("vertex_ai_safety_results", &metadata.safety_results),
            ("vertex_ai_citation_metadata", &metadata.citation_metadata),
        ] {
            body.insert(name.to_owned(), Json::List(values.clone()));
        }
    }
    if let Some(tier) = &response.service_tier {
        body.insert(String::from("service_tier"), Json::Text(tier.clone()));
    }
    Ok(body)
}

fn choices_body(choices: &Choices) -> Result<Json, ConversionError> {
    let Choices::Complete(complete) = choices else {
        return Err(ConversionError::StreamedChoices);
    };
    let mut rows = Vec::with_capacity(complete.len());
    for choice in complete {
        rows.push(choice_body(choice)?);
    }
    Ok(Json::List(rows))
}

fn choice_body(choice: &Choice) -> Result<Json, ConversionError> {
    let mut fields = IndexMap::new();
    fields.insert(
        String::from("finish_reason"),
        Json::Text(choice.finish_reason.to_owned()),
    );
    fields.insert(
        String::from("index"),
        Json::Number(Number::from(i64::from(choice.index))),
    );
    fields.insert(String::from("message"), message_body(&choice.message)?);
    if let Some(logprobs) = &choice.logprobs {
        let encoded = serde_json::to_value(logprobs).map_err(ConversionError::Encode)?;
        fields.insert(
            String::from("logprobs"),
            serde_json::from_value(encoded).map_err(ConversionError::Encode)?,
        );
    }
    if let Some(native) = &choice.native_finish_reason {
        let mut specific = IndexMap::new();
        specific.insert(
            String::from("native_finish_reason"),
            Json::Text(native.clone()),
        );
        fields.insert(
            String::from("provider_specific_fields"),
            Json::Object(specific),
        );
    }
    Ok(Json::Object(fields))
}

fn message_body(message: &Message) -> Result<Json, ConversionError> {
    let encoded = serde_json::to_value(message).map_err(ConversionError::Encode)?;
    let Json::Object(mut fields) =
        serde_json::from_value(encoded).map_err(ConversionError::Encode)?
    else {
        return Err(ConversionError::Shape);
    };
    for name in CONDITIONAL_MESSAGE_FIELDS {
        if fields.get(name) == Some(&Json::Null) {
            drop(fields.shift_remove(name));
        }
    }
    Ok(Json::Object(fields))
}

fn usage_body(usage: &Usage, blocked: bool) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        String::from("completion_tokens"),
        Json::Number(Number::from(usage.completion_tokens)),
    );
    fields.insert(
        String::from("prompt_tokens"),
        Json::Number(Number::from(usage.prompt_tokens)),
    );
    fields.insert(
        String::from("total_tokens"),
        Json::Number(Number::from(usage.total_tokens)),
    );
    let completion = usage
        .completion_tokens_details
        .as_ref()
        .map_or(Json::Null, completion_details);
    fields.insert(String::from("completion_tokens_details"), completion);
    let prompt = usage
        .prompt_tokens_details
        .as_ref()
        .map_or(Json::Null, prompt_details);
    fields.insert(String::from("prompt_tokens_details"), prompt);
    if !blocked {
        let cached = usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens);
        insert_nullable(&mut fields, "cache_read_input_tokens", cached);
    }
    Json::Object(fields)
}

fn completion_details(details: &CompletionTokensDetails) -> Json {
    let mut fields = IndexMap::new();
    insert_optional(
        &mut fields,
        "accepted_prediction_tokens",
        details.accepted_prediction_tokens,
    );
    insert_optional(&mut fields, "audio_tokens", details.audio_tokens);
    insert_optional(&mut fields, "reasoning_tokens", details.reasoning_tokens);
    insert_optional(
        &mut fields,
        "rejected_prediction_tokens",
        details.rejected_prediction_tokens,
    );
    insert_optional(&mut fields, "text_tokens", details.text_tokens);
    insert_optional(&mut fields, "image_tokens", details.image_tokens);
    insert_optional(&mut fields, "video_tokens", details.video_tokens);
    Json::Object(fields)
}

fn prompt_details(details: &PromptTokensDetails) -> Json {
    let mut fields = IndexMap::new();
    // the wrapper always holds these five, so a missing count stays null
    insert_nullable(&mut fields, "audio_tokens", details.audio_tokens);
    insert_nullable(&mut fields, "cached_tokens", details.cached_tokens);
    insert_nullable(&mut fields, "text_tokens", details.text_tokens);
    insert_nullable(&mut fields, "image_tokens", details.image_tokens);
    insert_nullable(&mut fields, "video_tokens", details.video_tokens);
    insert_optional(
        &mut fields,
        "web_search_requests",
        details.web_search_requests,
    );
    insert_optional(
        &mut fields,
        "google_maps_grounding_requests",
        details.google_maps_grounding_requests,
    );
    insert_optional(&mut fields, "tool_use_tokens", details.tool_use_tokens);
    Json::Object(fields)
}

fn insert_optional(fields: &mut IndexMap<String, Json>, name: &str, value: Option<i64>) {
    if let Some(number) = value {
        fields.insert(name.to_owned(), Json::Number(Number::from(number)));
    }
}

fn insert_nullable(fields: &mut IndexMap<String, Json>, name: &str, value: Option<i64>) {
    fields.insert(
        name.to_owned(),
        value.map_or(Json::Null, |number| Json::Number(Number::from(number))),
    );
}
