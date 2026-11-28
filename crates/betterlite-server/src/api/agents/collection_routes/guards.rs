use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_core::api::responses_input_content::is_truthy;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::Serialize;

pub const MISSING_KEY_DETAIL: &str = "Gemini managed-agent endpoints require a caller-supplied Gemini api_key (via 'litellm_params_template'). Falling back to the proxy's GOOGLE_API_KEY / GEMINI_API_KEY env vars is only permitted for proxy admins.";

#[derive(Serialize)]
struct DetailBody {
    detail: &'static str,
}

// the caller's own api_key is the only credential source a non-admin may use; admins keep the env fallback
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallerKeyMissing;

impl CallerKeyMissing {
    pub const STATUS: StatusCode = StatusCode::UNAUTHORIZED;
    pub const DETAIL: &'static str = MISSING_KEY_DETAIL;
}

impl IntoResponse for CallerKeyMissing {
    fn into_response(self) -> Response {
        (
            Self::STATUS,
            Json(DetailBody {
                detail: Self::DETAIL,
            }),
        )
            .into_response()
    }
}

pub fn enforce_provider_key(data: &JsonMap, principal: &Principal) -> Result<(), CallerKeyMissing> {
    if principal.roles.contains(&Role::PlatformAdmin) || is_truthy(data.get("api_key")) {
        return Ok(());
    }
    Err(CallerKeyMissing)
}
