use betterlite_openai::xai_tokens::codec::{Numeric, TokenPayload};
use betterlite_openai::xai_tokens::rules::{build_auth_record, is_expired, validate_xai_endpoint};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Expired {
        #[serde(with = "rust_decimal::serde::arbitrary_precision")]
        now: Decimal,
        expires_at: Option<Numeric>,
    },
    Endpoint(String),
    Record {
        #[serde(with = "rust_decimal::serde::arbitrary_precision")]
        now: Decimal,
        payload: TokenPayload,
        token_endpoint: String,
        fallback: Option<SecretString>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        Ok(match input {
            Input::Expired { now, expires_at } => {
                json!({ "expired": is_expired(*now, expires_at.as_ref()) })
            }
            Input::Endpoint(url) => json!({ "valid": validate_xai_endpoint(url).is_ok() }),
            Input::Record {
                now,
                payload,
                token_endpoint,
                fallback,
            } => {
                match build_auth_record(*now, payload.clone(), token_endpoint, fallback.as_ref()) {
                    Ok(record) => json!({ "record": record.stored() }),
                    Err(error) => json!({ "error": error.to_string() }),
                }
            }
        })
    }
}
