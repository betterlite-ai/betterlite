use serde::Serialize;
use thiserror::Error;

use crate::panw_prisma_airs::error_detail::{
    ErrorCode, ErrorDetail, ErrorType, GuardrailHttpError,
};

const NO_ARGUMENT_FIELD: &str = "MCP request blocked: no rewritable argument field present";
const INVALID_MCP_JSON: &str =
    "MCP request blocked: masked data is not valid JSON for structured arguments";

#[derive(Debug, Error)]
pub enum UnifiedError {
    #[error(transparent)]
    Scan(#[from] GuardrailHttpError),
    #[error("{NO_ARGUMENT_FIELD}")]
    McpArgumentsMissing,
    #[error("{INVALID_MCP_JSON}")]
    McpMaskedJson(#[source] serde_json::Error),
    #[error("the PANW Prisma AIRS tool event could not be encoded")]
    Encode(#[source] serde_json::Error),
}

impl UnifiedError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Scan(error) => error.status,
            Self::Encode(_) => 500,
            Self::McpArgumentsMissing | Self::McpMaskedJson(_) => 400,
        }
    }

    #[must_use]
    pub fn detail(&self) -> Option<BlockDetail<'_>> {
        match self {
            Self::Scan(error) => Some(BlockDetail::Scan(error.detail.as_ref())),
            Self::McpArgumentsMissing => Some(BlockDetail::Mcp(mcp_block(NO_ARGUMENT_FIELD))),
            Self::McpMaskedJson(_) => Some(BlockDetail::Mcp(mcp_block(INVALID_MCP_JSON))),
            Self::Encode(_) => None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum BlockDetail<'detail> {
    Scan(&'detail ErrorDetail),
    Mcp(McpBlockDetail),
}

#[derive(Debug, Serialize)]
pub struct McpBlockDetail {
    pub error: McpBlockBody,
}

#[derive(Debug, Serialize)]
pub struct McpBlockBody {
    pub message: &'static str,
    #[serde(rename = "type")]
    pub kind: ErrorType,
    pub code: ErrorCode,
}

const fn mcp_block(message: &'static str) -> McpBlockDetail {
    McpBlockDetail {
        error: McpBlockBody {
            message,
            kind: ErrorType::GuardrailViolation,
            code: ErrorCode::PanwPrismaAirsBlocked,
        },
    }
}
