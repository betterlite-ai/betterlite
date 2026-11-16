use std::error::Error as StdError;

use indexmap::IndexMap;
use thiserror::Error;

use crate::panw_prisma_airs::audit::Direction;
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::{
    ErrorBody, ErrorCode, ErrorDetail, ErrorType, GuardrailHttpError,
};
use crate::prisma_airs_masking::MaskingError;

// LiteLLM walks the request dict with .get and crashes on the wrong shape; this names the field instead.
#[derive(Debug, Error)]
pub enum HookDataError {
    #[error("the {field} field must be a list")]
    NotList { field: &'static str },
    #[error("the {field} field must be an object")]
    NotObject { field: &'static str },
    #[error("item {index} of {field} is not the expected object: {source}")]
    Item {
        field: &'static str,
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("item {index} of {field} is not text")]
    NotText { field: &'static str, index: usize },
    #[error("the scan marker {key} must be a boolean")]
    MarkerNotBoolean { key: String },
    #[error("the request metadata could not be read: {source}")]
    Metadata {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request messages could not be masked: {source}")]
    Masking {
        #[source]
        source: MaskingError,
    },
    #[error("the masked messages could not be written back: {source}")]
    Encoding {
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLM's catch-all except block: a scan that raises becomes a 500 with the request or response wording.
#[must_use]
pub fn internal(
    error: &dyn StdError,
    direction: Direction,
    guardrail: &PanwPrismaAirs,
) -> GuardrailHttpError {
    tracing::error!("PANW Prisma AIRS scan failed: {error}");
    let side = match direction {
        Direction::Request => "request",
        Direction::Response => "response",
    };
    GuardrailHttpError {
        status: 500,
        detail: Box::new(ErrorDetail {
            error: ErrorBody {
                action: None,
                scan_fields: IndexMap::new(),
                message: format!("Security scan failed - {side} blocked for safety"),
                kind: ErrorType::GuardrailScanError,
                code: ErrorCode::PanwPrismaAirsScanFailed,
                guardrail: guardrail.settings.guardrail_name.clone(),
                category: "api_error".to_owned(),
            },
        }),
    }
}
