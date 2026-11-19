use betterlite_anthropic::api::messages_errors::fields::Fields;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_thinking::response::decode_messages_response;
use betterlite_prompts::arguments::json::{Json as BodyJson, Object};
use serde_json::Number;
use serde_json::value::RawValue;

use super::error::MantleMessagesError;
use super::messages::is_zero_number;

const INPUT_TOKENS: &str = "input_tokens";
const OUTPUT_TOKENS: &str = "output_tokens";
const USAGE: &str = "usage";

// transform_anthropic_messages_response: B962's native decode keeps every field as sent, then the usage object gets
// the two required counts as zero defaults underneath whatever the provider returned.
pub fn transform_response(
    body: &[u8],
    status: u16,
    limits: &ApiLimits,
) -> Result<Fields, MantleMessagesError> {
    let mut fields = decode_messages_response(body, status, limits)?.fields;
    let usage = normalized_usage(fields.get(USAGE).map(Box::as_ref))?;
    fields.insert(USAGE.to_owned(), usage);
    Ok(fields)
}

fn normalized_usage(existing: Option<&RawValue>) -> Result<Box<RawValue>, MantleMessagesError> {
    let mut usage = Object::new();
    usage.insert(
        INPUT_TOKENS.to_owned(),
        BodyJson::Number(Number::from(0_u8)),
    );
    usage.insert(
        OUTPUT_TOKENS.to_owned(),
        BodyJson::Number(Number::from(0_u8)),
    );
    let Some(raw) = existing else {
        return encode(&usage);
    };
    match serde_json::from_str::<BodyJson>(raw.get())? {
        BodyJson::Object(entries) => {
            for (key, value) in entries {
                usage.insert(key, value);
            }
        }
        BodyJson::Null | BodyJson::Bool(false) => {}
        BodyJson::Number(number) if is_zero_number(&number) => {}
        BodyJson::Text(text) if text.is_empty() => {}
        BodyJson::List(items) if items.is_empty() => {}
        BodyJson::Bool(true) | BodyJson::Number(_) | BodyJson::Text(_) | BodyJson::List(_) => {
            return Err(MantleMessagesError::UsageNotMapping);
        }
    }
    encode(&usage)
}

fn encode(usage: &Object) -> Result<Box<RawValue>, MantleMessagesError> {
    Ok(RawValue::from_string(serde_json::to_string(usage)?)?)
}
