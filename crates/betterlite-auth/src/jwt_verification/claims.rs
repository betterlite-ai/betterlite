use betterlite_config::sections::jwt_settings::records::Selector;
use betterlite_paths::paths::Document;

use super::errors::VerificationError;
use super::settings::audience_candidates;

// pyjwt's verify_* options, the audience and issuer the caller configured, and the reading of the clock.
pub struct ClaimChecks<'config> {
    pub audience: Option<&'config Selector>,
    pub issuer: Option<&'config str>,
    pub verify_aud: bool,
    pub verify_iss: bool,
    pub now: f64,
}

// JWTHandler._decode_jwt_with_public_key delegates the claim set to pyjwt: iat, nbf, exp, iss, aud, sub, jti in order.
pub fn validate_claims(
    payload: &Document,
    checks: &ClaimChecks<'_>,
) -> Result<(), VerificationError> {
    validate_not_in_future(
        payload,
        checks.now,
        "iat",
        "Issued At claim (iat) must be an integer.",
    )?;
    validate_not_in_future(
        payload,
        checks.now,
        "nbf",
        "Not Before claim (nbf) must be an integer.",
    )?;
    validate_expiration(payload, checks.now)?;
    if checks.verify_iss {
        validate_issuer(payload, checks.issuer)?;
    }
    if checks.verify_aud {
        validate_audience(payload, checks.audience)?;
    }
    validate_string_claim(payload, "sub", "Subject must be a string")?;
    validate_string_claim(payload, "jti", "JWT ID must be a string")
}

fn numeric_claim(
    payload: &Document,
    claim: &str,
    message: &str,
) -> Result<Option<f64>, VerificationError> {
    let Some(value) = payload.json.get(claim) else {
        return Ok(None);
    };
    value
        .as_f64()
        .map(Some)
        .ok_or_else(|| VerificationError::Validation {
            detail: message.to_owned(),
        })
}

// pyjwt reads iat and nbf the same way: an optional integer that must not lie in the future.
fn validate_not_in_future(
    payload: &Document,
    now: f64,
    claim: &str,
    message: &str,
) -> Result<(), VerificationError> {
    let Some(value) = numeric_claim(payload, claim, message)? else {
        return Ok(());
    };
    if value > now {
        return Err(VerificationError::Validation {
            detail: format!("The token is not yet valid ({claim})"),
        });
    }
    Ok(())
}

// pyjwt runs the exp check only when the claim is present, so a signed token without exp has no expiry
fn validate_expiration(payload: &Document, now: f64) -> Result<(), VerificationError> {
    let Some(value) = payload.json.get("exp") else {
        return Ok(());
    };
    let expiration = value
        .as_f64()
        .ok_or_else(|| VerificationError::Validation {
            detail: "Expiration Time claim (exp) must be an integer.".to_owned(),
        })?;
    // leeway is zero: a token is expired from the instant its exp passes
    if expiration <= now {
        return Err(VerificationError::Expired);
    }
    Ok(())
}

fn validate_issuer(payload: &Document, issuer: Option<&str>) -> Result<(), VerificationError> {
    let Some(expected) = issuer else {
        return Ok(());
    };
    let Some(value) = payload.json.get("iss") else {
        return Err(VerificationError::Validation {
            detail: "Token is missing the \"iss\" claim".to_owned(),
        });
    };
    let Some(claim) = value.as_str() else {
        return Err(VerificationError::Validation {
            detail: "Issuer must be a string.".to_owned(),
        });
    };
    if claim != expected {
        return Err(VerificationError::Validation {
            detail: "Invalid issuer".to_owned(),
        });
    }
    Ok(())
}

// pyjwt's _validate_aud: a configured selector must match the claim, and with none any truthy claim is invalid
fn validate_audience(
    payload: &Document,
    audience: Option<&Selector>,
) -> Result<(), VerificationError> {
    let Some(configured) = audience else {
        return validate_unscoped_audience(payload);
    };
    let Some(value) = payload.json.get("aud") else {
        return Err(missing_audience());
    };
    if value.is_null()
        || value.as_str().is_some_and(str::is_empty)
        || value.as_array().is_some_and(Vec::is_empty)
    {
        return Err(missing_audience());
    }
    let candidates = audience_candidates(configured);
    let audience_claims = if let Some(items) = value.as_array() {
        let mut claims = Vec::with_capacity(items.len());
        for item in items {
            let Some(text) = item.as_str() else {
                return Err(invalid_audience_format());
            };
            claims.push(text);
        }
        claims
    } else {
        let Some(text) = value.as_str() else {
            return Err(invalid_audience_format());
        };
        vec![text]
    };
    if !candidates
        .iter()
        .any(|candidate| audience_claims.contains(&candidate.as_str()))
    {
        return Err(VerificationError::Validation {
            detail: "Invalid audience".to_owned(),
        });
    }
    Ok(())
}

// pyjwt with verify_aud on but no audience rejects any token that carries a truthy aud claim
fn validate_unscoped_audience(payload: &Document) -> Result<(), VerificationError> {
    if unscoped_audience_is_absent(payload) {
        return Ok(());
    }
    Err(VerificationError::Validation {
        detail: "Invalid audience".to_owned(),
    })
}

fn unscoped_audience_is_absent(payload: &Document) -> bool {
    let Some(value) = payload.json.get("aud") else {
        return true;
    };
    value.is_null()
        || value.as_str().is_some_and(str::is_empty)
        || value.as_array().is_some_and(Vec::is_empty)
        || matches!(value.as_bool(), Some(false))
        || matches!(value.as_i64(), Some(0))
}

fn missing_audience() -> VerificationError {
    VerificationError::Validation {
        detail: "Token is missing the \"aud\" claim".to_owned(),
    }
}

fn invalid_audience_format() -> VerificationError {
    VerificationError::Validation {
        detail: "Invalid claim format in token".to_owned(),
    }
}

fn validate_string_claim(
    payload: &Document,
    claim: &str,
    message: &str,
) -> Result<(), VerificationError> {
    let Some(value) = payload.json.get(claim) else {
        return Ok(());
    };
    if value.as_str().is_none() {
        return Err(VerificationError::Validation {
            detail: message.to_owned(),
        });
    }
    Ok(())
}
