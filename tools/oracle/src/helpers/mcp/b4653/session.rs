use serde::Deserialize;
use serde_json::json;

use betterlite_mcp::session_credentials::{self, SessionBearerResult, SessionRefreshResult};
use betterlite_mcp::session_tokens::mint_session_token;
use betterlite_mcp::types::{SessionKeys, SessionPrincipal, SessionSigningKeys, SessionTokenKind};
use secrecy::{ExposeSecret as _, SecretString};

use crate::helpers::{HelperError, HelperResult};

use super::{at, helper_error};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionFields {
    signing_key: String,
    open_signing_key: Option<String>,
    candidate: Option<String>,
    mint: Option<SessionMint>,
    open_now: i64,
    #[serde(default)]
    expected_client_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionMint {
    principal: SessionPrincipal,
    kind: SessionTokenKind,
    mint_now: i64,
}

fn session_keys(signing_key: &str) -> Result<SessionSigningKeys, Box<HelperError>> {
    let keys = SessionKeys::new(SecretString::from(signing_key));
    keys.map(SessionSigningKeys::Symmetric)
        .map_err(|error| helper_error("SessionKeyError", &error))
}

fn mint_session_candidate(
    mint: &SessionMint,
    keys: &SessionSigningKeys,
) -> Result<String, Box<HelperError>> {
    let now = at(mint.mint_now)?;
    let minted = mint_session_token(&mint.principal, mint.kind, keys, now, || Ok([0_u8; 16]))
        .map_err(|error| helper_error("SessionMintError", &error))?;
    Ok(minted.token.expose_secret().to_owned())
}

pub(crate) fn run_session_bearer(fields: &SessionFields) -> HelperResult {
    let mint_keys = session_keys(&fields.signing_key)?;
    let candidate = match (&fields.mint, &fields.candidate) {
        (Some(mint), _) => mint_session_candidate(mint, &mint_keys)?,
        (None, Some(literal)) => literal.clone(),
        (None, None) => {
            return Err(helper_error(
                "ValueError",
                &"either mint or candidate is required",
            ));
        }
    };
    let open_keys = match &fields.open_signing_key {
        Some(signing) => session_keys(signing)?,
        None => mint_keys,
    };
    let now = at(fields.open_now)?;
    let result = session_credentials::resolve_session_bearer(&candidate, &open_keys, now);
    Ok(match result {
        SessionBearerResult::NotSessionBearer => json!({ "tag": "not_session_bearer" }),
        SessionBearerResult::SessionBearerAdmitted { principal, .. } => json!({
            "tag": "admitted",
            "principal": principal,
        }),
        SessionBearerResult::SessionBearerInvalid { expired } => json!({
            "tag": "invalid",
            "expired": expired,
        }),
    })
}

pub(crate) fn run_session_refresh(fields: &SessionFields) -> HelperResult {
    let mint_keys = session_keys(&fields.signing_key)?;
    let candidate = match (&fields.mint, &fields.candidate) {
        (Some(mint), _) => mint_session_candidate(mint, &mint_keys)?,
        (None, Some(literal)) => literal.clone(),
        (None, None) => {
            return Err(helper_error(
                "ValueError",
                &"either mint or candidate is required",
            ));
        }
    };
    let open_keys = match &fields.open_signing_key {
        Some(signing) => session_keys(signing)?,
        None => mint_keys,
    };
    let now = at(fields.open_now)?;
    let expected_client_id = fields
        .expected_client_id
        .clone()
        .ok_or_else(|| helper_error("ValueError", &"expected_client_id is required"))?;
    let result = session_credentials::open_session_refresh_bearer(
        &candidate,
        &open_keys,
        now,
        &expected_client_id,
    );
    Ok(match result {
        SessionRefreshResult::SessionRefreshOpened { principal, jti } => json!({
            "tag": "opened",
            "principal": principal,
            "jti": jti,
        }),
        SessionRefreshResult::SessionRefreshInvalid => json!({ "tag": "invalid" }),
    })
}
