use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

use super::token::IdentityClaims;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationRecord {
    pub authority: String,
    pub client_id: String,
    pub home_account_id: String,
    pub tenant_id: String,
    pub username: String,
}

#[derive(Debug, Error)]
pub enum RecordError {
    #[error("the token response has no identity token")]
    IdentityTokenMissing,
    #[error("the identity token has no {claim}")]
    ClaimMissing { claim: &'static str },
    #[error("the client info is not base64")]
    ClientInfoBase64(#[from] base64::DecodeError),
    #[error("the client info is not the expected JSON")]
    ClientInfoJson(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct ClientInfo {
    uid: String,
    utid: String,
}

pub fn build(
    claims: Option<&IdentityClaims>,
    client_info: Option<&str>,
) -> Result<AuthenticationRecord, RecordError> {
    let identity = claims.ok_or(RecordError::IdentityTokenMissing)?;
    let home_account_id = match client_info {
        Some(raw) => client_info_id(raw)?,
        None => identity
            .sub
            .clone()
            .ok_or(RecordError::ClaimMissing { claim: "sub" })?,
    };
    let issuer = identity
        .iss
        .as_deref()
        .ok_or(RecordError::ClaimMissing { claim: "iss" })?;
    let (authority, path) = issuer_parts(issuer);
    let tenant_id = identity
        .tid
        .clone()
        .filter(|tenant| !tenant.is_empty())
        .unwrap_or(path);
    let username = identity
        .preferred_username
        .clone()
        .filter(|name| !name.is_empty())
        .or_else(|| identity.upn.clone())
        .ok_or(RecordError::ClaimMissing { claim: "upn" })?;
    let client_id = identity
        .aud
        .clone()
        .ok_or(RecordError::ClaimMissing { claim: "aud" })?;
    Ok(AuthenticationRecord {
        authority,
        client_id,
        home_account_id,
        tenant_id,
        username,
    })
}

fn client_info_id(raw: &str) -> Result<String, RecordError> {
    let bytes = URL_SAFE_NO_PAD.decode(raw.trim_end_matches('='))?;
    let info: ClientInfo = serde_json::from_slice(&bytes)?;
    Ok(format!("{}.{}", info.uid, info.utid))
}

fn issuer_parts(issuer: &str) -> (String, String) {
    match Url::parse(issuer) {
        Ok(url) => (
            url.host_str().unwrap_or_default().to_owned(),
            url.path().trim_matches('/').to_owned(),
        ),
        Err(_error) => (String::new(), issuer.trim_matches('/').to_owned()),
    }
}
