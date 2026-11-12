use betterlite_paths::error::PathError;
use betterlite_paths::paths::lookup::{find, find_or};
use betterlite_paths::paths::{Document, PathLimits};
use thiserror::Error;

use super::TypedValue;
use super::settings::JwtAuthSettings;
use super::trust::TrustClaims as _;

const CLAIM_PATH_LIMITS: PathLimits = PathLimits {
    max_path_bytes: 8192,
    max_depth: 256,
    max_selectors: 256,
    max_visits: 100_000,
};

#[derive(Debug, Error)]
pub enum ClaimError {
    #[error("the claim path {claim:?} is outside the dot-path limits")]
    Path {
        claim: String,
        #[source]
        source: PathError,
    },
    #[error("the claim {claim:?} does not hold the declared type")]
    Type {
        claim: String,
        #[source]
        source: serde_json::Error,
    },
}

pub struct JwtClaims<'auth> {
    pub claims: TypedValue,
    pub settings: &'auth JwtAuthSettings,
    pub trusted_issuers: &'auth [String],
}

impl JwtClaims<'_> {
    pub(super) fn nested(&self, path: &str) -> Result<Option<TypedValue>, ClaimError> {
        find(&CLAIM_PATH_LIMITS, &self.claims, path).map_err(|source| ClaimError::Path {
            claim: path.to_owned(),
            source,
        })
    }

    pub(super) fn nested_with_default(
        &self,
        path: &str,
        default: &str,
    ) -> Result<TypedValue, ClaimError> {
        let fallback = Document {
            json: serde_json::json!(default),
        };
        find_or(&CLAIM_PATH_LIMITS, &self.claims, path, fallback).map_err(|source| {
            ClaimError::Path {
                claim: path.to_owned(),
                source,
            }
        })
    }

    pub(super) fn nested_text(
        &self,
        path: &str,
        default: Option<&str>,
    ) -> Result<Option<String>, ClaimError> {
        let found = match default {
            Some(text) => Some(self.nested_with_default(path, text)?),
            None => self.nested(path)?,
        };
        found.map_or(Ok(None), |value| text_value(value, path))
    }
}

#[derive(Clone, Copy)]
pub(super) enum WhenUnset {
    UseDefault,
    Absent,
}

impl WhenUnset {
    pub(super) fn resolve(self, default: Option<&str>) -> Option<String> {
        match self {
            Self::UseDefault => default.map(str::to_owned),
            Self::Absent => None,
        }
    }
}

// the issuer-normalized claim for a trusted issuer, present even when its value is null
pub(super) fn marker_claim(
    claims: &JwtClaims<'_>,
    marker: &str,
) -> Result<Option<TypedValue>, ClaimError> {
    if claims.is_trusted_marker_token() && claims.claims.json.get(marker).is_some() {
        return claims.nested(marker);
    }
    Ok(None)
}

pub(super) fn field_value(
    claims: &JwtClaims<'_>,
    field: Option<&str>,
    default: Option<&str>,
    unset: WhenUnset,
) -> Result<Option<String>, ClaimError> {
    field.map_or_else(
        || Ok(unset.resolve(default)),
        |path| claims.nested_text(path, default),
    )
}

pub(super) fn list_first(value: &TypedValue, claim: &str) -> Result<Option<String>, ClaimError> {
    let Some(items) = value.json.as_array() else {
        return Ok(None);
    };
    let Some(first) = items.first() else {
        return Ok(None);
    };
    text_value(
        Document {
            json: first.clone(),
        },
        claim,
    )
}

pub(super) fn text_value(value: TypedValue, claim: &str) -> Result<Option<String>, ClaimError> {
    if value.json.is_null() {
        return Ok(None);
    }
    let text = serde_json::from_value::<String>(value.json).map_err(|source| ClaimError::Type {
        claim: claim.to_owned(),
        source,
    })?;
    Ok(Some(text))
}

pub(super) fn text_list(value: TypedValue, claim: &str) -> Result<Vec<String>, ClaimError> {
    serde_json::from_value::<Vec<String>>(value.json).map_err(|source| ClaimError::Type {
        claim: claim.to_owned(),
        source,
    })
}
