use super::{CohereEmbeddingError, TextEncoding};
use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

// the aliases are Cohere's meta.billed_units names; serialization keeps the OpenAI names
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct BilledUnits {
    #[serde(alias = "input_tokens")]
    pub text_tokens: Option<i64>,
    #[serde(alias = "images")]
    pub image_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CohereEmbeddingUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_tokens_details: Option<BilledUnits>,
}

#[derive(Default, Deserialize)]
struct Meta {
    billed_units: Option<BilledUnits>,
}

// _calculate_usage: only when the answer bills neither field does the encoding service count the input
pub fn calculate_usage(
    input: &[String],
    encoding: &dyn TextEncoding,
    meta: Option<&Json>,
) -> Result<CohereEmbeddingUsage, CohereEmbeddingError> {
    let parsed: Option<Meta> = serde_json::from_value(serde_json::to_value(meta)?)?;
    let billed = parsed.unwrap_or_default().billed_units.unwrap_or_default();
    let (prompt_tokens, prompt_tokens_details) = if billed == BilledUnits::default() {
        let mut counted = 0_usize;
        for text in input {
            counted = counted
                .checked_add(encoding.count_tokens(text)?)
                .ok_or(CohereEmbeddingError::UsageOverflow)?;
        }
        let prompt_tokens =
            i64::try_from(counted).map_err(|_overflow| CohereEmbeddingError::UsageOverflow)?;
        (prompt_tokens, None)
    } else {
        let billed_total = billed
            .text_tokens
            .unwrap_or(0)
            .checked_add(billed.image_tokens.unwrap_or(0))
            .ok_or(CohereEmbeddingError::UsageOverflow)?;
        (billed_total, Some(billed))
    };
    Ok(CohereEmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_tokens_details,
    })
}
