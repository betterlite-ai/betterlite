use betterlite_models::decisions::Json;
use serde::Serialize;

use super::error::PartnerCountError;

pub const TOKENIZER_USED: &str = "vertex_ai_partner_models";

// LiteLLM returns exactly these two fields, and a reply without input_tokens counts zero
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PartnerCount {
    pub input_tokens: i64,
    pub tokenizer_used: &'static str,
}

pub fn decode_partner_count(bytes: &[u8]) -> Result<PartnerCount, PartnerCountError> {
    let value: Json = serde_json::from_slice(bytes).map_err(PartnerCountError::Decode)?;
    let Json::Object(mut fields) = value else {
        return Err(PartnerCountError::NotAnObject);
    };
    let input_tokens = match fields.shift_remove("input_tokens") {
        None | Some(Json::Null) => 0,
        Some(Json::Number(number)) => number.as_i64().ok_or(PartnerCountError::InputTokens)?,
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(PartnerCountError::InputTokens);
        }
    };
    Ok(PartnerCount {
        input_tokens,
        tokenizer_used: TOKENIZER_USED,
    })
}
