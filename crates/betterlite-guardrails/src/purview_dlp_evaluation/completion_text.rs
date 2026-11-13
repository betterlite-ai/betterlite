use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::purview_dlp_evaluation::responses_extraction::responses_function_call_args;
use crate::purview_graph_client::tool_call_extraction::tool_call_arguments;

pub enum CompletionResponse<'response> {
    TextCompletion(&'response Object),
    Responses(&'response ResponsesApiResponse),
    Model(&'response Object),
}

fn push_text(value: Option<&Json>, parts: &mut Vec<String>) {
    let Some(Json::Text(text)) = value else {
        return;
    };
    if !text.trim().is_empty() {
        parts.push(text.clone());
    }
}

fn choice_fields(response: &Object) -> Vec<&Object> {
    let Some(Json::List(choices)) = response.get("choices") else {
        return Vec::new();
    };
    choices
        .iter()
        .filter_map(|choice| match choice {
            Json::Object(fields) => Some(fields),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

fn text_completion_parts(response: &Object) -> Vec<String> {
    let mut parts = Vec::new();
    for choice in choice_fields(response) {
        push_text(choice.get("text"), &mut parts);
    }
    parts
}

fn responses_api_parts(response: &ResponsesApiResponse) -> Vec<String> {
    let mut parts = Vec::new();
    let text = response.output_text();
    if !text.trim().is_empty() {
        parts.push(text);
    }
    parts.extend(responses_function_call_args(response));
    parts
}

fn model_response_parts(response: &Object) -> Vec<String> {
    let mut parts = Vec::new();
    for choice in choice_fields(response) {
        let Some(Json::Object(message)) = choice.get("message") else {
            continue;
        };
        push_text(message.get("content"), &mut parts);
        parts.extend(tool_call_arguments(message));
    }
    parts
}

#[must_use]
pub fn completion_response_text_parts(response: &CompletionResponse<'_>) -> Vec<String> {
    match response {
        CompletionResponse::TextCompletion(body) => text_completion_parts(body),
        CompletionResponse::Responses(body) => responses_api_parts(body),
        CompletionResponse::Model(body) => model_response_parts(body),
    }
}

mod tests;
