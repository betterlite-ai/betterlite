use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use serde_json::json;
use uuid::Uuid;

use crate::helpers::{HelperCall, HelperError, invoke};

use super::failure;

pub(super) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    pub(super) fn parse(values: &[String]) -> Result<Self, Box<HelperError>> {
        let parsed = values
            .iter()
            .map(|value| Uuid::parse_str(value))
            .collect::<Result<VecDeque<Uuid>, _>>()
            .map_err(|error| failure(error.to_string()))?;
        Ok(Self {
            values: Mutex::new(parsed),
        })
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_else(Uuid::nil)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the token_count helper failed: {message}")]
pub(super) struct TokenCountFailure {
    message: String,
}

pub(super) struct TokenizerCounter;

impl ReasoningTokenCounter for TokenizerCounter {
    type Failure = TokenCountFailure;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Self::Failure> {
        let call = HelperCall {
            operation: "token_count".to_owned(),
            input: json!({
                "function": "token_counter",
                "model": "",
                "text": text,
                "count_response_tokens": true,
            }),
        };
        let output = invoke(&call);
        if let Some(error) = output.error {
            return Err(TokenCountFailure {
                message: error.message,
            });
        }
        let Some(value) = output.value else {
            return Err(TokenCountFailure {
                message: "the token_count helper returned no count".to_owned(),
            });
        };
        value
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .ok_or_else(|| TokenCountFailure {
                message: "the token_count helper returned a non-integer count".to_owned(),
            })
    }
}
