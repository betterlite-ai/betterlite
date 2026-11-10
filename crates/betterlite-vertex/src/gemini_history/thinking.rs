use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_history::content::{Part, TextPart};

// A thinking string is replayed as a signed text part beyond this size instead of parsed
pub const MAX_THINKING_BYTES: usize = 0x0010_0000;
const SIGNATURE_KEY: &str = "thoughtSignature";

#[must_use]
pub fn decode_thinking_part(thinking: &str, signature: &str) -> Part {
    if let Some(object) = decoded_object(thinking, signature) {
        return Part::Decoded(object);
    }
    Part::Text(TextPart {
        text: thinking.to_owned(),
        signature: Some(Json::Text(signature.to_owned())),
    })
}

fn decoded_object(thinking: &str, signature: &str) -> Option<Object> {
    if thinking.len() > MAX_THINKING_BYTES {
        return None;
    }
    let Ok(Json::Object(mut object)) = serde_json::from_str::<Json>(thinking) else {
        return None;
    };
    // LiteLLM's duplicate keyword raises and falls back to the signed text too
    if object.contains_key(SIGNATURE_KEY) {
        return None;
    }
    object.insert(SIGNATURE_KEY.to_owned(), Json::Text(signature.to_owned()));
    Some(object)
}
