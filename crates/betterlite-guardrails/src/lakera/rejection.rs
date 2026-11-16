use reqwest::StatusCode;
use serde::Serialize;

use super::response::LakeraModerationResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LakeraViolation {
    Jailbreak,
    PromptInjection,
    ContentSafetyPolicy,
}

impl LakeraViolation {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Jailbreak => "Violated jailbreak threshold",
            Self::PromptInjection => "Violated prompt_injection threshold",
            Self::ContentSafetyPolicy => "Violated content safety policy",
        }
    }
}

// the dict LiteLLM puts in HTTPException.detail
#[derive(Debug, Clone, Serialize)]
pub struct LakeraRejectionDetail {
    pub error: &'static str,
    pub lakera_ai_response: LakeraModerationResponse,
}

#[derive(Debug, Clone)]
pub struct LakeraRejection {
    pub status: StatusCode,
    pub detail: LakeraRejectionDetail,
}

impl LakeraRejection {
    // LiteLLM raises HTTPException(400, detail={"error": ..., "lakera_ai_response": response})
    pub(super) fn new(violation: LakeraViolation, response: &LakeraModerationResponse) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            detail: LakeraRejectionDetail {
                error: violation.message(),
                lakera_ai_response: response.clone(),
            },
        }
    }
}
