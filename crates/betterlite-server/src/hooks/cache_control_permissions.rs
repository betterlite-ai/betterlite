use axum::http::StatusCode;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use std::future::ready;

mod tests;

#[derive(Debug, Clone, Deserialize)]
pub struct CacheControlCallData {
    #[serde(default)]
    pub cache: Option<JsonData>,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheControlPermissionsError {
    #[error("Not allowed to set {key} as a cache control. Contact admin to change permissions.")]
    Forbidden { key: String },
}

impl CacheControlPermissionsError {
    // LiteLLM raises HTTPException(status_code=403, ...) here; betterlite returns a typed error instead.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
        }
    }
}

pub struct CacheControlPermissions;

impl CacheControlPermissions {
    pub async fn async_pre_call_hook(
        &self,
        user_api_key_dict: &UserAPIKeyAuth,
        _cache: Option<&JsonData>,
        data: &CacheControlCallData,
        _call_type: &str,
    ) -> Result<(), CacheControlPermissionsError> {
        let Some(JsonData::Object(cache_args)) = &data.cache else {
            return ready(Ok(())).await;
        };

        let allowed_cache_controls = match allowed_cache_controls(user_api_key_dict) {
            Ok(allowed_cache_controls) => allowed_cache_controls,
            Err(source) => {
                // Mirrors LiteLLM's catch-all: log at debug and permit the request rather than fail it.
                tracing::debug!(error = %source, "cache control permissions: could not read the authenticated principal");
                return ready(Ok(())).await;
            }
        };
        if allowed_cache_controls.is_empty() {
            return ready(Ok(())).await;
        }

        for key in cache_args.keys() {
            if !allowed_cache_controls.contains(key) {
                return ready(Err(CacheControlPermissionsError::Forbidden {
                    key: key.clone(),
                }))
                .await;
            }
        }
        ready(Ok(())).await
    }
}

#[derive(Debug, Deserialize)]
struct AuthenticatedPrincipalView {
    #[serde(default)]
    allowed_cache_controls: Option<Vec<String>>,
}

fn allowed_cache_controls(
    user_api_key_dict: &UserAPIKeyAuth,
) -> Result<Vec<String>, serde_json::Error> {
    let text = serde_json::to_string(user_api_key_dict)?;
    let principal: AuthenticatedPrincipalView = serde_json::from_str(&text)?;
    Ok(principal.allowed_cache_controls.unwrap_or_default())
}
