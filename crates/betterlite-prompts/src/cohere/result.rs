use crate::arguments::json::{Json, Object};
use crate::cohere::content::parts_text;
use crate::cohere::error::CohereError;
use crate::cohere::tool_calls::{Recovery, recovered_call};
use crate::cohere::types::CohereToolResult;
use crate::message_content::{Content, Message};

const FUNCTION: &str = "function";
const RESULT: &str = "result";

pub(crate) fn tool_result(
    message: &Message,
    recovery: &mut Recovery<'_>,
    index: usize,
) -> Result<CohereToolResult, CohereError> {
    let content = outputs_content(message, index)?;
    let mut call = recovered_call(message, recovery.tool_calls, &mut recovery.budget)?;
    if message.role == FUNCTION {
        call.name = message
            .name
            .clone()
            .ok_or(CohereError::MissingMessageName { index })?;
    }
    Ok(CohereToolResult {
        call,
        outputs: vec![content],
    })
}

fn outputs_content(message: &Message, index: usize) -> Result<Json, CohereError> {
    let text = match &message.content {
        Some(Content::Text(text)) => text.clone(),
        Some(Content::Parts(parts)) => parts_text(parts, index)?,
        None => String::new(),
    };
    if text.is_empty() {
        return Ok(Json::Object(Object::new()));
    }
    let Ok(parsed) = serde_json::from_str::<Json>(&text) else {
        let mut result = Object::new();
        result.insert(RESULT.to_owned(), Json::Text(text));
        return Ok(Json::Object(result));
    };
    Ok(parsed)
}
