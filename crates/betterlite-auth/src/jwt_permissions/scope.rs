use betterlite_config::sections::jwt_settings::records::ScopeMapping;

use crate::auth_errors::conversion::AuthFailure;
use crate::auth_model_patterns::denial::model_denied_client_message;

const FORBIDDEN: u16 = 403;

// check_scope_based_access: every mapping whose scope the token carries contributes its nonempty models
pub fn check_scope_based_access(
    scope_mappings: Option<&[ScopeMapping]>,
    scopes: &[String],
    model: Option<&str>,
) -> Result<(), Box<AuthFailure>> {
    let Some(mappings) = scope_mappings.filter(|mappings| !mappings.is_empty()) else {
        return Ok(());
    };
    let mut allowed_models = Vec::new();
    for mapping in mappings {
        if !scopes.iter().any(|scope| scope == &mapping.scope) {
            continue;
        }
        let Some(models) = mapping.models.as_deref() else {
            continue;
        };
        allowed_models.extend(models.iter().map(String::as_str));
    }
    // request_data.get("model") is falsy for an absent model and for the empty string
    let Some(requested) = model.filter(|candidate| !candidate.is_empty()) else {
        return Ok(());
    };
    if allowed_models.contains(&requested) {
        return Ok(());
    }
    Err(Box::new(AuthFailure::ModelAccessDenied {
        detail: model_denied_client_message(requested),
        internal_message: format!(
            "model={requested} not allowed. Allowed_models={allowed_models:?}"
        ),
        status_code: FORBIDDEN,
    }))
}
