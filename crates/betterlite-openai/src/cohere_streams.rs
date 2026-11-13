use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::num::ParseIntError;
use std::str::{self, Utf8Error};

use rust_decimal::{Decimal, Error as DecimalError};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Error as JsonError, value::RawValue};
use thiserror::Error;

use crate::error::{ProviderError, ProviderErrorParts};
use wire::{Index, ProviderFields, Record, ToolCall};

mod tests;
mod v2;
pub mod wire;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CohereStreamVersion {
    V1,
    V2,
}

#[derive(Debug, Error)]
pub enum CohereStreamError {
    #[error("invalid UTF-8 in Cohere stream: {0}")]
    Utf8(#[from] Utf8Error),
    #[error("invalid Cohere stream JSON: {0}")]
    Json(#[from] JsonError),
    #[error("invalid Cohere stream index: {source}")]
    InvalidIndex {
        #[source]
        source: ParseIntError,
    },
    #[error("invalid numeric Cohere index: {0}")]
    NumericIndex(#[from] DecimalError),
    #[error("invalid separator in Cohere index")]
    IndexSeparator,
    #[error("Cohere v2 transformation failed: {source}")]
    V2Transform {
        #[source]
        source: Box<Self>,
    },
    #[error("Cohere stream token total exceeds the supported range")]
    UsageOverflow,
    #[error("Cohere completion has no finish_reason")]
    MissingFinishReason,
    #[error("Cohere stream line exceeds the byte limit")]
    TooLarge,
}

#[derive(Debug, Error)]
pub enum CohereIteratorError {
    #[error("Error receiving chunk from stream: {source}")]
    Receiving {
        #[source]
        source: IoError,
    },
    #[error("Error parsing chunk: {source}")]
    Parsing {
        #[source]
        source: CohereStreamError,
    },
    #[error("Cohere chunk transformation failed: {source}")]
    Transformation {
        #[source]
        source: CohereStreamError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CohereUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct CohereChunk {
    pub text: String,
    pub tool_use: Option<ToolCall>,
    pub is_finished: bool,
    pub finish_reason: String,
    pub usage: Option<CohereUsage>,
    pub index: i64,
    pub provider_specific_fields: Option<ProviderFields>,
}

pub fn validate_environment(headers: &mut BTreeMap<String, String>, api_key: &SecretString) {
    headers.insert(
        "Request-Source".to_owned(),
        "unspecified:litellm".to_owned(),
    );
    headers.insert("accept".to_owned(), "application/json".to_owned());
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    if !api_key.expose_secret().is_empty() {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", api_key.expose_secret()),
        );
    }
}

#[must_use]
pub fn cohere_error(status_code: u16, message: String) -> ProviderError {
    let parts = ProviderErrorParts {
        status_code,
        message,
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    };
    ProviderError::from(parts)
}

pub fn decode_cohere_line(
    version: CohereStreamVersion,
    input: &[u8],
    is_bytes: bool,
) -> Result<CohereChunk, CohereStreamError> {
    if input.len() > 0x0010_0000 {
        return Err(CohereStreamError::TooLarge);
    }
    let decoded = str::from_utf8(input)?;
    let text = if is_bytes {
        decoded
            .find("data:")
            .and_then(|position| decoded.get(position..))
            .unwrap_or(decoded)
    } else {
        decoded
    };
    let raw: &RawValue = serde_json::from_str(text)?;
    let result = transform(version, raw);
    match version {
        CohereStreamVersion::V1 => result,
        CohereStreamVersion::V2 => result.map_err(|source| CohereStreamError::V2Transform {
            source: Box::new(source),
        }),
    }
}

fn transform(
    version: CohereStreamVersion,
    raw: &RawValue,
) -> Result<CohereChunk, CohereStreamError> {
    let record: Record = serde_json::from_str(raw.get())?;
    let index = parse_index(record.index.as_ref())?;
    let mut chunk = CohereChunk {
        text: String::new(),
        tool_use: None,
        is_finished: false,
        finish_reason: String::new(),
        usage: None,
        index,
        provider_specific_fields: None,
    };
    match version {
        CohereStreamVersion::V1 => parse_v1(record, &mut chunk)?,
        CohereStreamVersion::V2 => v2::parse_v2(record, &mut chunk)?,
    }
    Ok(chunk)
}

fn parse_index(index: Option<&Index>) -> Result<i64, CohereStreamError> {
    match index {
        Some(Index::Number(number)) => {
            let decimal = number.to_string().parse::<Decimal>()?;
            Ok(i64::try_from(decimal)?)
        }
        Some(Index::Boolean(value)) => Ok(i64::from(*value)),
        Some(Index::Text(text)) => {
            let trimmed = text.trim();
            let digits = trimmed
                .strip_prefix('+')
                .or_else(|| trimmed.strip_prefix('-'))
                .unwrap_or(trimmed);
            if digits.split('_').any(str::is_empty) {
                return Err(CohereStreamError::IndexSeparator);
            }
            trimmed
                .replace('_', "")
                .parse()
                .map_err(|source| CohereStreamError::InvalidIndex { source })
        }
        None => Ok(0),
    }
}

fn parse_v1(record: Record, chunk: &mut CohereChunk) -> Result<(), CohereStreamError> {
    match (record.text, record.is_finished) {
        (Some(text), _) => chunk.text = text,
        (None, Some(true)) => {
            chunk.is_finished = true;
            chunk.finish_reason = record
                .finish_reason
                .ok_or(CohereStreamError::MissingFinishReason)?;
        }
        (None, Some(false) | None) => {}
    }
    chunk.provider_specific_fields = record.citations.map(|citations| ProviderFields {
        tool_plan: None,
        citations: Some(citations),
    });
    Ok(())
}

pub fn decode_cohere_item(
    version: CohereStreamVersion,
    item: Result<Option<Vec<u8>>, IoError>,
) -> Result<Option<CohereChunk>, CohereIteratorError> {
    let Some(bytes) = item.map_err(|source| CohereIteratorError::Receiving { source })? else {
        return Ok(None);
    };
    decode_cohere_line(version, &bytes, false)
        .map(Some)
        .map_err(|source| {
            if matches!(&source, CohereStreamError::MissingFinishReason)
                || matches!(&source, CohereStreamError::Json(error) if error.is_data())
            {
                CohereIteratorError::Transformation { source }
            } else {
                CohereIteratorError::Parsing { source }
            }
        })
}
