use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::straiker::v3::error::V3Error;
use crate::straiker::v3::messages::conversation_messages;

const MESSAGE_SEPARATOR: u8 = 0x1e;

// A fingerprint of the conversation after each of its messages, first to last. The last one names the conversation as sent; the earlier ones let a request that carries a blocked exchange as its history be recognised, not only an exact resend. A `prompt` or a string `input` has one fingerprint.
pub fn conversation_prefixes(request_body: &Object) -> Result<Vec<String>, V3Error> {
    let messages = conversation_messages(request_body);
    if !messages.is_empty() {
        let mut digest = Sha256::new();
        let mut prefixes = Vec::with_capacity(messages.len());
        for message in messages {
            digest.update(sorted_json(message)?.as_bytes());
            digest.update([MESSAGE_SEPARATOR]);
            prefixes.push(hex::encode(digest.clone().finalize()));
        }
        return Ok(prefixes);
    }
    let plain = if request_body.contains_key("input") {
        request_body.get("input")
    } else {
        request_body.get("prompt")
    };
    let Some(value) = plain.filter(|value| !matches!(value, Json::Null)) else {
        return Ok(Vec::new());
    };
    Ok(vec![hex::encode(Sha256::digest(
        sorted_json(value)?.as_bytes(),
    ))])
}

fn sorted_json<Item>(value: &Item) -> Result<String, V3Error>
where
    Item: Serialize,
{
    let mut tree = serde_json::to_value(value).map_err(V3Error::Serialize)?;
    tree.sort_all_objects();
    serde_json::to_string(&tree).map_err(V3Error::Serialize)
}

mod tests;
