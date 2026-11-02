use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{ContentPart, Message};
use betterlite_prompts::reasoning_fields::base64_data::extract_base64_data;
use betterlite_prompts::reasoning_fields::content_split::{
    extract_reasoning_content, parse_content_for_reasoning,
};
use betterlite_prompts::reasoning_fields::encrypted_signature::{
    encrypted_content_of_signature, encrypted_reasoning_signature, is_encrypted_reasoning_block,
    is_unsignable_thinking_block,
};
use betterlite_prompts::reasoning_fields::replay_groups::reasoning_items_from_blocks;
use betterlite_prompts::reasoning_fields::strip_encrypted::strip_encrypted_reasoning;
use betterlite_prompts::reasoning_fields::thinking_text::reasoning_content_from_blocks;
use serde::Deserialize;
use serde_json::{json, to_value};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "reasoning_content_from_thinking_blocks")]
    ThinkingText { thinking_blocks: Vec<ContentPart> },
    #[serde(rename = "encrypted_reasoning_signature")]
    Sign { encrypted_content: String },
    #[serde(rename = "encrypted_content_from_signature")]
    Unsign { signature: Json },
    #[serde(rename = "is_encrypted_reasoning_block")]
    IsEncrypted { block: ContentPart },
    #[serde(rename = "is_unsignable_thinking_block")]
    IsUnsignable { block: ContentPart },
    #[serde(rename = "strip_encrypted_reasoning_from_messages")]
    Strip { messages: Vec<Message> },
    #[serde(rename = "responses_reasoning_items_from_thinking_blocks")]
    ReplayGroups { thinking_blocks: Vec<ContentPart> },
    #[serde(rename = "extract_base64_data")]
    Base64 { image_url: String },
    #[serde(rename = "extract_reasoning_content")]
    SplitMessage { message: Message },
    #[serde(rename = "parse_content_for_reasoning")]
    SplitText {
        #[serde(default)]
        message_text: Option<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ThinkingText { thinking_blocks } => {
                ok_value(reasoning_content_from_blocks(thinking_blocks))
            }
            Input::Sign { encrypted_content } => {
                ok_value(encrypted_reasoning_signature(encrypted_content))
            }
            Input::Unsign { signature } => ok_value(encrypted_content_of_signature(signature)),
            Input::IsEncrypted { block } => ok_value(is_encrypted_reasoning_block(block)),
            Input::IsUnsignable { block } => ok_value(is_unsignable_thinking_block(block)),
            Input::Strip { messages } => ok_value(strip_encrypted_reasoning(messages, None)),
            Input::ReplayGroups { thinking_blocks } => {
                ok_value(reasoning_items_from_blocks(thinking_blocks))
            }
            Input::Base64 { image_url } => ok_value(extract_base64_data(image_url)),
            Input::SplitMessage { message } => {
                let (reasoning_content, content) = extract_reasoning_content(message);
                ok_value(json!({"reasoning_content": reasoning_content, "content": content}))
            }
            Input::SplitText { message_text } => {
                let (reasoning_content, content) =
                    parse_content_for_reasoning(message_text.as_deref());
                ok_value(json!({"reasoning_content": reasoning_content, "content": content}))
            }
        }
    }
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    match to_value(payload) {
        Ok(json) => Ok(json),
        Err(error) => Err(classified_error(error.to_string())),
    }
}

fn classified_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
