use betterlite_config::sections::jwt_settings::records::IssuerConfig;
use betterlite_models::decisions::Json;
use betterlite_paths::error::PathError;
use betterlite_paths::paths::lookup::find;
use betterlite_paths::paths::{Document, PathLimits};
use thiserror::Error;

use super::errors::VerificationError;
use crate::passthrough::caller_headers::header_sets::LITELLM_INTERNAL_CLAIMS;

const CLAIM_PATH_LIMITS: PathLimits = PathLimits {
    max_path_bytes: 8192,
    max_depth: 256,
    max_selectors: 256,
    max_visits: 100_000,
};

pub(crate) const LITELLM_JWT_ISSUER_CLAIM: &str = "_litellm_jwt_issuer";
pub(crate) const LITELLM_USER_ID_CLAIM: &str = "_litellm_user_id";
pub(crate) const LITELLM_USER_EMAIL_CLAIM: &str = "_litellm_user_email";
pub(crate) const LITELLM_TEAM_ID_CLAIM: &str = "_litellm_team_id";
pub(crate) const LITELLM_TEAM_IDS_CLAIM: &str = "_litellm_team_ids";
pub(crate) const LITELLM_ORG_ID_CLAIM: &str = "_litellm_org_id";
pub(crate) const LITELLM_END_USER_ID_CLAIM: &str = "_litellm_end_user_id";

#[derive(Debug, Error)]
pub enum ClaimError {
    #[error("the verified payload is not a JSON object")]
    NotObject,
    #[error("the claim path {claim:?} is outside the dot-path limits")]
    Path {
        claim: String,
        #[source]
        source: PathError,
    },
    #[error("the verified payload could not be rebuilt as a typed document")]
    Conversion(#[source] serde_json::Error),
}

impl From<ClaimError> for VerificationError {
    fn from(error: ClaimError) -> Self {
        Self::Validation {
            detail: error.to_string(),
        }
    }
}

// auth_jwt's global path: exactly LiteLLM's seven internal names leave the verified payload.
pub fn strip_internal_claims(payload: &Document) -> Result<Document, ClaimError> {
    let mut stripped = payload.clone();
    let Some(fields) = stripped.json.as_object_mut() else {
        return Err(ClaimError::NotObject);
    };
    for claim in LITELLM_INTERNAL_CLAIMS {
        fields.shift_remove(claim);
    }
    Ok(stripped)
}

// _apply_issuer_claim_mappings: strip, stamp the configured issuer, then map six claims from the original payload.
pub fn normalize_with_issuer(
    payload: &Document,
    config: &IssuerConfig,
) -> Result<Document, ClaimError> {
    let mut normalized = strip_internal_claims(payload)?;
    let Some(fields) = normalized.json.as_object_mut() else {
        return Err(ClaimError::NotObject);
    };
    let stamped =
        serde_json::to_value(Json::Text(config.issuer.clone())).map_err(ClaimError::Conversion)?;
    fields.insert(LITELLM_JWT_ISSUER_CLAIM.to_owned(), stamped);
    let mappings = [
        (&config.user_id_jwt_field, LITELLM_USER_ID_CLAIM),
        (&config.user_email_jwt_field, LITELLM_USER_EMAIL_CLAIM),
        (&config.team_id_jwt_field, LITELLM_TEAM_ID_CLAIM),
        (&config.team_ids_jwt_field, LITELLM_TEAM_IDS_CLAIM),
        (&config.org_id_jwt_field, LITELLM_ORG_ID_CLAIM),
        (&config.end_user_id_jwt_field, LITELLM_END_USER_ID_CLAIM),
    ];
    for (source_path, target) in mappings {
        let Some(path) = source_path.as_deref() else {
            continue;
        };
        // every mapping reads the original payload, so one mapped field never feeds another
        let found = find(&CLAIM_PATH_LIMITS, payload, path).map_err(|source| ClaimError::Path {
            claim: path.to_owned(),
            source,
        })?;
        let Some(found) = found else {
            continue;
        };
        if found.json.is_null() || found.json.as_str() == Some("") {
            continue;
        }
        fields.insert(target.to_owned(), found.json);
    }
    Ok(normalized)
}
