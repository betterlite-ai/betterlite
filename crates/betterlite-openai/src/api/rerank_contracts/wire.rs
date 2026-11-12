use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use thiserror::Error;

use super::records::{RerankRequest, RerankResponse};
use crate::api::responses_shapes::wire::{
    ShapeDecodeError, WireDecodeLimits, decode_responses_shapes,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RerankRecordKind {
    Request = 0,
    Response = 1,
}

impl TryFrom<u8> for RerankRecordKind {
    type Error = RerankContractError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Request),
            1 => Ok(Self::Response),
            _ => Err(RerankContractError::UnknownKind(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RerankContract {
    Request(RerankRequest),
    Response(RerankResponse),
}

#[derive(Debug, Error)]
pub enum RerankContractError {
    #[error("unknown rerank record kind: {0}")]
    UnknownKind(u8),
    #[error("the rerank record could not be decoded: {0}")]
    Shape(#[from] ShapeDecodeError),
    #[error("the rerank record does not match its kind: {0}")]
    Json(#[from] serde_json::Error),
    #[error("score cannot be represented as an exact decimal")]
    ScoreOutOfRange(#[source] DecimalLexemeError),
}

#[derive(Debug, PartialEq, Eq)]
pub struct ErrorDetails<'error> {
    pub status: Option<u16>,
    pub error_type: Option<&'static str>,
    pub param: Option<&'static str>,
    pub code: Option<&'static str>,
    pub location: Option<&'error str>,
}

impl RerankContractError {
    #[must_use]
    pub const fn details(&self) -> ErrorDetails<'_> {
        let (status, error_type, param, code, location) = match self {
            Self::ScoreOutOfRange(_) => (
                Some(502),
                Some("bad_gateway"),
                Some("score"),
                Some("decimal_out_of_range"),
                None,
            ),
            Self::UnknownKind(_) | Self::Shape(_) | Self::Json(_) => (None, None, None, None, None),
        };
        ErrorDetails {
            status,
            error_type,
            param,
            code,
            location,
        }
    }
}

pub fn decode_rerank_contracts(
    bytes: &[u8],
    kind: RerankRecordKind,
    limits: WireDecodeLimits,
) -> Result<RerankContract, RerankContractError> {
    match kind {
        RerankRecordKind::Request => Ok(RerankContract::Request(decode_responses_shapes(
            bytes, limits,
        )?)),
        RerankRecordKind::Response => {
            let value: Json = decode_responses_shapes(bytes, limits)?;
            validate_scores(&value)?;
            let raw = serde_json::to_value(&value)?;
            Ok(RerankContract::Response(serde_json::from_value(raw)?))
        }
    }
}

fn validate_scores(value: &Json) -> Result<(), RerankContractError> {
    let Json::Object(root) = value else {
        return Ok(());
    };
    let Some(Json::List(results)) = root.get("results") else {
        return Ok(());
    };
    for result in results {
        let Json::Object(fields) = result else {
            continue;
        };
        if let Some(Json::Number(number)) = fields.get("relevance_score") {
            parse_decimal_lexeme(&number.to_string())
                .map_err(RerankContractError::ScoreOutOfRange)?;
        }
    }
    Ok(())
}
