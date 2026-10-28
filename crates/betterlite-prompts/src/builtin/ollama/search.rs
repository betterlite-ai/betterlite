use super::super::{Field, Message, PromptError, text};
use super::json;

pub(super) fn joined(message: &Message) -> Result<String, PromptError> {
    let mut output = text::joined(&message.content);
    let Some(Field::List(results)) = message.extra.get("search_results") else {
        return Ok(output);
    };
    for result in results {
        let Field::Object(fields) = result else {
            continue;
        };
        for key in ["source", "title"] {
            if let Some(Field::Text(text)) = fields.get(key) {
                output.push_str(text);
            }
        }
        let blocks = match fields.get("content") {
            Some(Field::List(blocks)) => blocks.as_slice(),
            _ => &[],
        };
        for block in blocks {
            if let Field::Object(part) = block
                && let Some(Field::Text(text)) = part.get("text")
            {
                output.push_str(text);
            }
        }
        if let Some(citations) = fields.get("citations")
            && citations != &Field::Null
        {
            output.push_str(&json::render(citations, true)?);
        }
    }
    Ok(output)
}
