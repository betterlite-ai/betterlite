use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use super::{CohereEmbeddingError, TextEncoding};

// PromptTokensDetailsWrapper: both billed fields survive a zero and stay null when the answer omits them
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BilledUnits {
    pub text_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CohereEmbeddingUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_tokens_details: Option<BilledUnits>,
}

// _calculate_usage: only when the answer bills neither field does the encoding service count the input
pub fn calculate_usage(
    input: &[String],
    encoding: &dyn TextEncoding,
    meta: Option<&Json>,
) -> Result<CohereEmbeddingUsage, CohereEmbeddingError> {
    let meta_fields = match meta {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => {
            return Err(CohereEmbeddingError::Field {
                name: "meta",
                problem: "is not an object",
            });
        }
    };
    let billed = match meta_fields.and_then(|fields| fields.get("billed_units")) {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => {
            return Err(CohereEmbeddingError::Field {
                name: "meta.billed_units",
                problem: "is not an object",
            });
        }
    };
    let text_tokens = optional_integer(
        billed.and_then(|fields| fields.get("input_tokens")),
        "meta.billed_units.input_tokens",
    )?;
    let image_tokens = optional_integer(
        billed.and_then(|fields| fields.get("images")),
        "meta.billed_units.images",
    )?;
    if text_tokens.is_none() && image_tokens.is_none() {
        let mut counted = 0usize;
        for text in input {
            let tokens = encoding
                .count_tokens(text)
                .map_err(|source| CohereEmbeddingError::Encoding { source })?;
            counted = counted
                .checked_add(tokens)
                .ok_or(CohereEmbeddingError::UsageOverflow)?;
        }
        let prompt_tokens =
            i64::try_from(counted).map_err(|_overflow| CohereEmbeddingError::UsageOverflow)?;
        return Ok(CohereEmbeddingUsage {
            prompt_tokens,
            completion_tokens: 0,
            total_tokens: prompt_tokens,
            prompt_tokens_details: None,
        });
    }
    let prompt_tokens = text_tokens
        .unwrap_or(0)
        .checked_add(image_tokens.unwrap_or(0))
        .ok_or(CohereEmbeddingError::UsageOverflow)?;
    Ok(CohereEmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_tokens_details: Some(BilledUnits {
            text_tokens,
            image_tokens,
        }),
    })
}

fn optional_integer(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<i64>, CohereEmbeddingError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            number
                .as_i64()
                .map(Some)
                .ok_or(CohereEmbeddingError::Field {
                    name: field,
                    problem: "is not an integer",
                })
        }
        Some(_) => Err(CohereEmbeddingError::Field {
            name: field,
            problem: "is not an integer",
        }),
    }
}
