use crate::arguments::json::{Json, Object};

use super::blocks::{is_blank, text_block};
use super::error::ProcessorError;

const REASONING_CONTENT: &str = "reasoningContent";
const REASONING_TEXT: &str = "reasoningText";
const SIGNATURE: &str = "signature";
const TEXT: &str = "text";
const THINKING: &str = "thinking";

enum Reasoning<'block> {
    Keep,
    Text { text: &'block str },
}

fn reasoning_text_field(fields: &Object, block: usize) -> Result<&str, ProcessorError> {
    match fields.get(TEXT) {
        None => Err(ProcessorError::ReasoningTextWithoutText { block }),
        Some(Json::Text(text)) => Ok(text.as_str()),
        Some(_) => Err(ProcessorError::ReasoningTextNotText { block }),
    }
}

fn reasoning_state(block: &Json, index: usize) -> Result<Reasoning<'_>, ProcessorError> {
    let fields = match block {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(ProcessorError::ReasoningNotObject { block: index });
        }
    };
    let reasoning_content = match fields.get(REASONING_CONTENT) {
        None | Some(Json::Null) => return Ok(Reasoning::Keep),
        Some(Json::Object(content)) => content,
        Some(_) => return Err(ProcessorError::ReasoningContentNotObject { block: index }),
    };
    let text_fields = match reasoning_content.get(REASONING_TEXT) {
        None | Some(Json::Null) => return Ok(Reasoning::Keep),
        Some(Json::Object(object)) if object.is_empty() => return Ok(Reasoning::Keep),
        Some(Json::Object(object)) => object,
        Some(_) => return Err(ProcessorError::ReasoningTextNotObject { block: index }),
    };
    let signed = match text_fields.get(SIGNATURE) {
        None | Some(Json::Null) => false,
        Some(Json::Text(signature)) => !signature.is_empty(),
        Some(_) => return Err(ProcessorError::ReasoningSignatureNotText { block: index }),
    };
    if signed {
        return Ok(Reasoning::Keep);
    }
    Ok(Reasoning::Text {
        text: reasoning_text_field(text_fields, index)?,
    })
}

pub fn translate_thinking_blocks(thinking_blocks: &[Json]) -> Result<Vec<Json>, ProcessorError> {
    let mut blocks = Vec::with_capacity(thinking_blocks.len());
    for (index, block) in thinking_blocks.iter().enumerate() {
        let fields = match block {
            Json::Object(fields) => fields,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                return Err(ProcessorError::ThinkingNotObject { block: index });
            }
        };
        let text = match fields.get(THINKING) {
            None | Some(Json::Null) => "",
            Some(Json::Text(text)) => text.as_str(),
            Some(_) => return Err(ProcessorError::ThinkingNotText { block: index }),
        };
        let mut reasoning = Object::from_iter([(TEXT.to_owned(), Json::Text(text.to_owned()))]);
        match fields.get(SIGNATURE) {
            None | Some(Json::Null) => {}
            Some(Json::Text(signature)) => {
                reasoning.insert(SIGNATURE.to_owned(), Json::Text(signature.clone()));
            }
            Some(_) => {
                return Err(ProcessorError::ThinkingSignatureNotText { block: index });
            }
        }
        blocks.push(Json::Object(Object::from_iter([(
            REASONING_CONTENT.to_owned(),
            Json::Object(Object::from_iter([(
                REASONING_TEXT.to_owned(),
                Json::Object(reasoning),
            )])),
        )])));
    }
    Ok(blocks)
}

pub fn append_thinking_blocks(
    thinking_blocks: &[Json],
    assistant_parts: &mut Vec<Json>,
) -> Result<(), ProcessorError> {
    let mut filtered: Vec<Json> = Vec::new();
    for (index, block) in thinking_blocks.iter().enumerate() {
        match reasoning_state(block, index)? {
            Reasoning::Keep => filtered.push(block.clone()),
            Reasoning::Text { text } => {
                if !is_blank(text) {
                    assistant_parts.push(text_block(text));
                }
            }
        }
    }
    assistant_parts.append(&mut filtered);
    Ok(())
}
