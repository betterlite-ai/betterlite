use serde::Deserialize;

use crate::google_count_tokens::error::GoogleCountError;

// a count reply carries a few counters; anything past this is not one
pub const MAX_COUNT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCountResponse {
    pub total_tokens: Option<u64>,
    pub total_billable_characters: Option<u64>,
    pub prompt_tokens_details: Option<Vec<ModalityTokens>>,
    #[serde(rename = "tokenizer_used")]
    pub tokenizer_used: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModalityTokens {
    pub modality: String,
    pub token_count: u64,
}

pub fn decode_google_count(bytes: &[u8]) -> Result<GoogleCountResponse, GoogleCountError> {
    if bytes.len() > MAX_COUNT_BYTES {
        return Err(GoogleCountError::ResponseTooLarge {
            limit: MAX_COUNT_BYTES,
        });
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let decoded = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        GoogleCountError::Json {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })?;
    deserializer
        .end()
        .map_err(|source| GoogleCountError::Json {
            path: "the document root".to_owned(),
            source,
        })?;
    Ok(decoded)
}
