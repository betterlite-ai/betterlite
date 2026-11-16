use betterlite_prompts::arguments::json::{Json, Object};

use crate::straiker::v3::prompt_text::{PromptTokenDecoder, prompt_texts};

// A legacy completion is relayed as user turns, one per prompt text, so Straiker reads it as a conversation.
#[must_use]
pub fn prompt_as_messages(prompt: &Json, decoder: &dyn PromptTokenDecoder) -> Option<Vec<Object>> {
    let texts = prompt_texts(prompt, decoder)?;
    Some(
        texts
            .into_iter()
            .map(|text| {
                Object::from([
                    ("role".to_owned(), Json::Text("user".to_owned())),
                    ("content".to_owned(), Json::Text(text)),
                ])
            })
            .collect(),
    )
}

mod tests;
