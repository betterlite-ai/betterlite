use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use chrono::{DateTime, TimeDelta, Utc};

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::rows::server_id_of;
use crate::search_tools::convert::isoformat;
use crate::stored_cipher::DbCipher;

pub mod decode;
pub mod rows;
pub mod store;
pub mod types;

mod tests;

use decode::{decode_user_credential, decoded_row, parse_oauth_payload};
use rows::{CredentialRows, CredentialScope, stored_text, text_field};
use store::{guard_byok_overwrite, store_user_credential};
use types::{
    CredentialError, CredentialType, McpUserCredentialListItem, OAuthCredentialPayload,
    OAuthGrantState, StoreOAuthCredential,
};

// LiteLLM's store_user_oauth_credential: one JSON payload encrypted into the BYOK column, with a connected_at and expires_at from the one `now` the caller read
pub async fn store_user_oauth_credential<A>(
    table: &A,
    cipher: &DbCipher,
    request: &StoreOAuthCredential<'_>,
    now: DateTime<Utc>,
) -> Result<(), CredentialError>
where
    A: CredentialRows,
{
    if !request.skip_byok_guard {
        let existing = table
            .find_credential(request.user_id, request.server_id)
            .await?;
        guard_byok_overwrite(
            cipher,
            existing.as_ref(),
            request.user_id,
            request.server_id,
        )?;
    }
    let payload = oauth_payload(request, now)?;
    let encoded = serde_json::to_string(&payload).map_err(CredentialError::Json)?;
    store_user_credential(table, cipher, request.user_id, request.server_id, &encoded).await
}

// the payload LiteLLM builds and json.dumps: only present optionals are written, in the same field set
fn oauth_payload(
    request: &StoreOAuthCredential<'_>,
    now: DateTime<Utc>,
) -> Result<OAuthCredentialPayload, CredentialError> {
    let expires_at = request
        .expires_in
        .map(|seconds| moment_after(now, seconds).map(isoformat))
        .transpose()?;
    let payload = OAuthCredentialPayload {
        access_token: Some(request.access_token.clone()),
        credential_type: CredentialType::Oauth2,
        refresh_token: request.refresh_token.cloned(),
        expires_at,
        connected_at: Some(isoformat(now)),
        scopes: request.scopes.map(<[String]>::to_vec),
        identity_binding_proof: request.identity_binding_proof.map(str::to_owned),
        server_id: None,
    };
    Ok(payload)
}

fn moment_after(now: DateTime<Utc>, seconds: i64) -> Result<DateTime<Utc>, CredentialError> {
    let offset =
        TimeDelta::try_seconds(seconds).ok_or(CredentialError::ExpiresInOutOfRange { seconds })?;
    now.checked_add_signed(offset)
        .ok_or(CredentialError::ExpiresInOutOfRange { seconds })
}

// LiteLLM's is_oauth_credential_expired: a naive timestamp is UTC, an absent or unparsable one never counts as expired, and now + buffer > expires is the comparison, so a token expiring exactly at the deadline is still live
#[must_use]
pub fn is_oauth_credential_expired(
    credential: &OAuthCredentialPayload,
    now: DateTime<Utc>,
    buffer_seconds: i64,
) -> bool {
    let Some(expires) = credential.expires_at.as_deref().and_then(decode_billed_at) else {
        return false;
    };
    let Some(buffer) = TimeDelta::try_seconds(buffer_seconds) else {
        return false;
    };
    now.checked_add_signed(buffer)
        .is_some_and(|deadline| deadline > expires)
}

// LiteLLM's oauth_grant_state: a token inside the refresh buffer is not valid, and only a stored refresh token makes it refreshable; the caller owns the buffer setting
#[must_use]
pub fn oauth_grant_state(
    credential: Option<&OAuthCredentialPayload>,
    now: DateTime<Utc>,
    buffer_seconds: i64,
) -> OAuthGrantState {
    let Some(payload) = credential else {
        return OAuthGrantState::Absent;
    };
    if payload.access_token.is_none() {
        return OAuthGrantState::Absent;
    }
    if !is_oauth_credential_expired(payload, now, buffer_seconds) {
        return OAuthGrantState::Valid;
    }
    if payload.refresh_token.is_some() {
        return OAuthGrantState::Refreshable;
    }
    OAuthGrantState::Absent
}

// LiteLLM's _server_user_credential_item: a row whose payload is not OAuth2 is a BYOK key, and neither shape carries the secret; a payload that claims oauth2 with a mistyped field is the Json error
pub fn server_user_credential_item(
    row: &JsonData,
    cipher: &DbCipher,
) -> Result<McpUserCredentialListItem, CredentialError> {
    let user_id =
        text_field(row, "user_id").ok_or(CredentialError::MissingField { field: "user_id" })?;
    let updated_at = text_field(row, "updated_at").ok_or(CredentialError::MissingField {
        field: "updated_at",
    })?;
    let decoded = decode_user_credential(cipher, stored_text(row)?);
    let (credential_type, expires_at, connected_at) = match parse_oauth_payload(decoded.as_ref())? {
        Some(payload) => (
            CredentialType::Oauth2,
            payload.expires_at,
            payload.connected_at,
        ),
        None => (CredentialType::Byok, None, None),
    };
    Ok(McpUserCredentialListItem {
        user_id: user_id.to_owned(),
        credential_type,
        expires_at,
        connected_at,
        updated_at: updated_at.to_owned(),
    })
}

// LiteLLM's list_server_user_credentials: every user's credential for one server, summed up without the secret
#[rustfmt::skip]
pub async fn list_server_user_credentials<A>(table: &A, cipher: &DbCipher, server_id: &str) -> Result<Vec<McpUserCredentialListItem>, CredentialError> where A: CredentialRows {
    table
        .find_credentials(CredentialScope::Server(server_id))
        .await?
        .iter()
        .map(|row| server_user_credential_item(row, cipher))
        .collect()
}

// LiteLLM's list_user_oauth_credentials: a row that does not decrypt is warned about and skipped, a row that is not OAuth2 is skipped, and the payload is tagged with its server_id
#[rustfmt::skip]
pub async fn list_user_oauth_credentials<A>(table: &A, cipher: &DbCipher, user_id: &str) -> Result<Vec<OAuthCredentialPayload>, CredentialError> where A: CredentialRows {
    oauth_payloads(
        &table.find_credentials(CredentialScope::User(user_id)).await?,
        cipher,
        user_id,
    )
}

fn oauth_payloads(
    rows: &[JsonData],
    cipher: &DbCipher,
    user_id: &str,
) -> Result<Vec<OAuthCredentialPayload>, CredentialError> {
    let mut payloads = Vec::new();
    for row in rows {
        if let Some(payload) = user_oauth_payload(row, cipher, user_id)? {
            payloads.push(payload);
        }
    }
    Ok(payloads)
}

fn user_oauth_payload(
    row: &JsonData,
    cipher: &DbCipher,
    user_id: &str,
) -> Result<Option<OAuthCredentialPayload>, CredentialError> {
    let server_id = server_id_of(row)?;
    let decoded = decoded_row(row, cipher, user_id, &server_id)?;
    let Some(mut payload) = parse_oauth_payload(decoded.as_ref())? else {
        return Ok(None);
    };
    payload.server_id = Some(server_id);
    Ok(Some(payload))
}
