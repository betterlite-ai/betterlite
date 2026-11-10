use betterlite_anthropic::message_replay::advisor::strip_advisor_blocks;
use betterlite_anthropic::message_replay::cache_control::normalize_cache_control;
use betterlite_anthropic::message_replay::provider_fields::strip_provider_specific_fields;
use betterlite_anthropic::message_replay::tool_ids::sanitize_tool_use_ids;
use betterlite_anthropic::message_replay::{
    BlockStrip, EmptyKind, is_empty_block, is_empty_unsigned_thinking, is_invalid_thinking_error,
    strip_blocks, strip_thinking_from_request,
};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Advisor {
        messages: Vec<Json>,
        #[serde(default)]
        replace_with_text: bool,
    },
    InvalidThinkingError {
        texts: Vec<String>,
        requests: Vec<Object>,
    },
    StripEncrypted {
        messages: Vec<Json>,
    },
    EmptyPredicates {
        blocks: Vec<Json>,
    },
    StripEmptyContent {
        messages: Vec<Json>,
    },
    ToolUseIds {
        messages: Vec<Json>,
    },
    ProviderFields {
        messages: Vec<Json>,
    },
    CacheControl {
        payload: Object,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::Advisor {
                messages,
                replace_with_text,
            } => advisor(messages, *replace_with_text),
            Input::InvalidThinkingError { texts, requests } => thinking_error(texts, requests),
            Input::StripEncrypted { messages } => {
                Json::List(strip_blocks(messages, BlockStrip::EncryptedReasoning))
            }
            Input::EmptyPredicates { blocks } => empty_predicates(blocks),
            Input::StripEmptyContent { messages } => {
                Json::List(strip_blocks(messages, BlockStrip::EmptyContent))
            }
            Input::ToolUseIds { messages } => Json::List(sanitize_tool_use_ids(messages)),
            Input::ProviderFields { messages } => {
                Json::List(strip_provider_specific_fields(messages))
            }
            Input::CacheControl { payload } => Json::Object(normalize_cache_control(payload)),
        };
        encode(value)
    }
}

fn advisor(messages: &[Json], replace_with_text: bool) -> Json {
    let mut cleaned = messages.to_vec();
    strip_advisor_blocks(&mut cleaned, replace_with_text);
    Json::List(cleaned)
}

fn thinking_error(texts: &[String], requests: &[Object]) -> Json {
    let matches = texts
        .iter()
        .map(|text| Json::Bool(is_invalid_thinking_error(text)))
        .collect();
    let repaired = requests
        .iter()
        .map(|request| {
            let mut data = request.clone();
            strip_thinking_from_request(&mut data);
            Json::Object(data)
        })
        .collect();
    Json::Object(Object::from_iter([
        ("matches".to_owned(), Json::List(matches)),
        ("repaired".to_owned(), Json::List(repaired)),
    ]))
}

fn empty_predicates(blocks: &[Json]) -> Json {
    let thinking = blocks
        .iter()
        .map(|block| Json::Bool(is_empty_block(block, EmptyKind::Thinking)))
        .collect();
    let unsigned = blocks
        .iter()
        .map(|block| Json::Bool(is_empty_unsigned_thinking(block)))
        .collect();
    let text = blocks
        .iter()
        .map(|block| Json::Bool(is_empty_block(block, EmptyKind::Text)))
        .collect();
    Json::Object(Object::from_iter([
        ("thinking".to_owned(), Json::List(thinking)),
        ("unsigned".to_owned(), Json::List(unsigned)),
        ("text".to_owned(), Json::List(text)),
    ]))
}

fn encode(value: Json) -> crate::helpers::HelperResult {
    serde_json::to_value(value).map_err(|failure| {
        Box::new(crate::helpers::HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    })
}

mod tests;
