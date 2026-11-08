use betterlite_models::decisions::Json;
use regex::Regex;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use thiserror::Error;

const DEFAULT_MAX_BASE64_LENGTH: i64 = 64;
const MAX_TRUNCATION_DEPTH: usize = 20;
const BYTES_PER_KIB: u32 = 0x0400;
const BYTES_PER_MIB: u32 = 0x0010_0000;

#[derive(Debug, Error)]
pub enum Base64Error {
    #[error("the base64 data URI pattern could not be compiled")]
    Pattern(#[from] regex::Error),
    #[error("a base64 payload is too long to describe")]
    LengthOverflow,
    #[error("a base64 payload size overflowed the decimal range")]
    SizeOverflow,
    #[error(transparent)]
    Env(#[from] betterlite_env::EnvError),
}

pub struct Base64Truncator {
    data_uri: Regex,
    max_payload_length: usize,
}

impl Base64Truncator {
    pub fn new(max_payload_length: usize) -> Result<Self, Base64Error> {
        Ok(Self {
            data_uri: Regex::new("data:([^;]+);base64,([A-Za-z0-9+/=]+)")?,
            max_payload_length,
        })
    }

    // MAX_BASE64_LENGTH_FOR_LOGGING defaults to 64, and a value of 0 or below turns truncation off
    pub fn from_env(raw: Option<&str>) -> Result<Self, Base64Error> {
        let configured = match raw {
            Some(text) => betterlite_env::number::<i64>("MAX_BASE64_LENGTH_FOR_LOGGING", text)?,
            None => None,
        };
        let limit = configured.unwrap_or(DEFAULT_MAX_BASE64_LENGTH);
        Self::new(usize::try_from(limit).unwrap_or(0))
    }

    pub fn truncate_messages(&self, messages: &Json) -> Result<Json, Base64Error> {
        if self.max_payload_length == 0 {
            return Ok(messages.clone());
        }
        self.truncate_value(messages, 0)
    }

    fn truncate_value(&self, value: &Json, depth: usize) -> Result<Json, Base64Error> {
        let child_depth = depth.saturating_add(1);
        match value {
            Json::Text(text) => Ok(Json::Text(self.truncate_string(text)?)),
            Json::List(items) if depth <= MAX_TRUNCATION_DEPTH => items
                .iter()
                .map(|item| self.truncate_value(item, child_depth))
                .collect::<Result<_, _>>()
                .map(Json::List),
            Json::Object(fields) if depth <= MAX_TRUNCATION_DEPTH => fields
                .iter()
                .map(|(key, item)| Ok((key.clone(), self.truncate_value(item, child_depth)?)))
                .collect::<Result<_, _>>()
                .map(Json::Object),
            Json::List(_) | Json::Object(_) | Json::Null | Json::Bool(_) | Json::Number(_) => {
                Ok(value.clone())
            }
        }
    }

    fn truncate_string(&self, text: &str) -> Result<String, Base64Error> {
        let mut output = String::with_capacity(text.len());
        let mut copied_until = 0_usize;
        for captures in self.data_uri.captures_iter(text) {
            let (Some(whole), Some(mime), Some(payload)) =
                (captures.get(0), captures.get(1), captures.get(2))
            else {
                continue;
            };
            output.push_str(text.get(copied_until..whole.start()).unwrap_or_default());
            if payload.as_str().len() <= self.max_payload_length {
                output.push_str(whole.as_str());
            } else {
                let size = format_base64_size(payload.as_str().len())?;
                let marker = format!(
                    "data:{};base64,[base64_data truncated: {size}]",
                    mime.as_str()
                );
                output.push_str(&marker);
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
    for (unit, places, suffix) in [(BYTES_PER_MIB, 2, "MB"), (BYTES_PER_KIB, 1, "KB")] {
        if bytes >= Decimal::from(unit) {
            let mut scaled = bytes
                .checked_div(Decimal::from(unit))
                .ok_or(Base64Error::SizeOverflow)?
                .round_dp_with_strategy(places, RoundingStrategy::MidpointNearestEven);
            scaled.rescale(places);
            return Ok(format!("{scaled}{suffix}"));
        }
    }
    Ok(format!("{}B", bytes.trunc()))
}

mod tests;
