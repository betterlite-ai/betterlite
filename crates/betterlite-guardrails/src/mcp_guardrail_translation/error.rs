use reqwest::StatusCode;
use thiserror::Error;

use super::nesting_depth::{LeafPath, MAX_SCAN_DEPTH};
#[derive(Debug, Error)]
pub enum TranslationError<GuardrailFailure> {
    #[error(transparent)]
    Blocked(#[from] ContentBlocked),
    #[error("the guardrail failed on the MCP payload")]
    Guardrail(#[source] GuardrailFailure),
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContentBlocked {
    #[error(
        "content blocked: the {} nests deeper than {MAX_SCAN_DEPTH} levels, so the guardrail cannot scan it",
        .payload.label()
    )]
    TooDeep { payload: NestedPayload },
    #[error(
        "content blocked: the guardrail returned {returned} texts for {scanned} {}, so the redaction cannot be mapped back",
        .target.label()
    )]
    TextCount {
        returned: usize,
        scanned: usize,
        target: ScanTarget,
    },
    #[error(
        "content blocked: two guardrails running in parallel rewrote the same MCP tool call argument ({}) differently; remove run_in_parallel from one of them",
        .paths.iter().map(LeafPath::dotted).collect::<Vec<_>>().join(", ")
    )]
    ConflictingRewrite { paths: Vec<LeafPath> },
    #[error(
        "content blocked: the guardrail matched a structuredContent key or number in the MCP tool result, and those cannot be redacted"
    )]
    UnrewritableLabel,
}
impl ContentBlocked {
    pub const STATUS: StatusCode = StatusCode::BAD_REQUEST;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedPayload {
    ToolArguments,
    InputSchema,
    StructuredContent,
}
impl NestedPayload {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ToolArguments => "MCP tool call arguments",
            Self::InputSchema => "MCP tool input schema",
            Self::StructuredContent => "MCP tool result structuredContent",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTarget {
    ToolCall,
    ToolResult,
}
impl ScanTarget {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ToolCall => "MCP tool strings",
            Self::ToolResult => "MCP tool result texts",
        }
    }
}
