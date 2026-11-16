use crate::model_armor::decisions::model_response::{CallResponse, ResponseChoice};

use super::masking::changed_text;

// LiteLLM rewrites the text of every complete choice; a streamed delta belongs to B3735
pub(super) fn apply(response: &mut CallResponse, content: &str, sanitized: Option<String>) {
    let Some(text) = changed_text(content, sanitized) else {
        return;
    };
    let CallResponse::Model(model) = response else {
        return;
    };
    for choice in &mut model.choices {
        match choice {
            ResponseChoice::Complete(complete) if !complete.message.content.is_empty() => {
                complete.message.content.clone_from(&text);
            }
            ResponseChoice::Complete(_) | ResponseChoice::Streaming(_) => {}
        }
    }
}

mod tests;
