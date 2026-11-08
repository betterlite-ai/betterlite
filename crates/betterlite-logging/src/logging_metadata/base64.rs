use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use regex::Regex;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use thiserror::Error;

const MAX_TRUNCATION_DEPTH: usize = 20;
const BYTES_PER_KIB: u32 = 0x0400;
const BYTES_PER_MIB: u32 = 0x0010_0000;

#[derive(Debug, Error)]
pub enum Base64Error {
    #[error("the base64 data URI pattern could not be compiled")]
    Pattern {
        #[source]
        source: regex::Error,
    },
    #[error("a base64 payload is too long to describe")]
    LengthOverflow,
    #[error("a base64 payload size overflowed the decimal range")]
    SizeOverflow,
}

pub struct Base64Truncator {
    data_uri: Regex,
    max_payload_length: usize,
}

impl Base64Truncator {
    pub fn new(max_payload_length: usize) -> Result<Self, Base64Error> {
        let data_uri = Regex::new("data:([^;]+);base64,([A-Za-z0-9+/=]+)")
            .map_err(|source| Base64Error::Pattern { source })?;
        Ok(Self {
            data_uri,
            max_payload_length,
        })
    }

    pub fn truncate_messages(&self, messages: &Json) -> Result<Json, Base64Error> {
        if self.max_payload_length == 0 {
            return Ok(messages.clone());
        }
        self.truncate_value(messages, 0)
    }

    fn truncate_value(&self, value: &Json, depth: usize) -> Result<Json, Base64Error> {
        match value {
            Json::Text(text) => Ok(Json::Text(self.truncate_string(text)?)),
            Json::List(items) => {
                if depth > MAX_TRUNCATION_DEPTH {
                    return Ok(value.clone());
                }
                let mut truncated = Vec::with_capacity(items.len());
                for item in items {
                    truncated.push(self.truncate_value(item, depth.saturating_add(1))?);
                }
                Ok(Json::List(truncated))
            }
            Json::Object(fields) => {
                if depth > MAX_TRUNCATION_DEPTH {
                    return Ok(value.clone());
                }
                let mut truncated = IndexMap::with_capacity(fields.len());
                for (key, item) in fields {
                    truncated.insert(
                        key.clone(),
                        self.truncate_value(item, depth.saturating_add(1))?,
                    );
                }
                Ok(Json::Object(truncated))
            }
            Json::Null | Json::Bool(_) | Json::Number(_) => Ok(value.clone()),
        }
    }

    fn truncate_string(&self, text: &str) -> Result<String, Base64Error> {
        let mut output = String::with_capacity(text.len());
        let mut copied_until = 0_usize;
        for captures in self.data_uri.captures_iter(text) {
            let Some(whole) = captures.get(0) else {
                continue;
            };
            let (Some(mime), Some(payload)) = (captures.get(1), captures.get(2)) else {
                continue;
            };
            output.push_str(text.get(copied_until..whole.start()).unwrap_or_default());
            if payload.as_str().len() <= self.max_payload_length {
                output.push_str(whole.as_str());
            } else {
                output.push_str("data:");
                output.push_str(mime.as_str());
                output.push_str(";base64,[base64_data truncated: ");
                output.push_str(&format_base64_size(payload.as_str().len())?);
                output.push(']');
            }
            copied_until = whole.end();
        }
        output.push_str(text.get(copied_until..).unwrap_or_default());
        Ok(output)
    }
}

fn format_base64_size(num_chars: usize) -> Result<String, Base64Error> {
    let count = u64::try_from(num_chars).map_err(|_error| Base64Error::LengthOverflow)?;
    let bytes = Decimal::from(count)
        .checked_mul(Decimal::from(3_u32))
        .ok_or(Base64Error::SizeOverflow)?
        .checked_div(Decimal::from(4_u32))
        .ok_or(Base64Error::SizeOverflow)?;
    let mebibyte = Decimal::from(BYTES_PER_MIB);
    if bytes >= mebibyte {
        let megabytes = bytes
            .checked_div(mebibyte)
            .ok_or(Base64Error::SizeOverflow)?;
        let rounded = megabytes.round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
        return Ok(format!("{rounded:.2}MB"));
    }
    let kibibyte = Decimal::from(BYTES_PER_KIB);
    if bytes >= kibibyte {
        let kilobytes = bytes
            .checked_div(kibibyte)
            .ok_or(Base64Error::SizeOverflow)?;
        let rounded = kilobytes.round_dp_with_strategy(1, RoundingStrategy::MidpointNearestEven);
        return Ok(format!("{rounded:.1}KB"));
    }
    Ok(format!("{}B", bytes.trunc()))
}

mod tests;
