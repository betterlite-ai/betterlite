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

use decode::{
    decode_oauth_payload, decode_user_credential, parse_oauth_payload,
    warn_undecryptable_credential,
};
use rows::{CredentialRows, CredentialScope, text_field};
use store::{guard_byok_overwrite, store_user_credential};
use types::{
    CredentialError, CredentialType, McpUserCredentialListItem, OAuthCredentialPayload,
    OAuthGrantState, StoreOAuthCredential,
};

// LiteLLM's store_user_oauth_credential: one JSON payload encrypted into the BYOK column, with a connected_at and expires_at from the one `now` the caller read
pub async fn store_user_oauth_credential<A: CredentialRows>(
    table: &A,
    cipher: &DbCipher,
    request: &StoreOAuthCredential<'_>,
    now: DateTime<Utc>,
) -> Result<(), CredentialError> {
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

// LiteLLM's is_oauth_credential_expired: a naive timestamp is UTC, an absent or unparseable one never counts as expired, and now + buffer > expires is the comparison, so a token expiring exactly at the deadline is still live
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
    let Some(credential) = credential else {
        return OAuthGrantState::Absent;
    };
    if credential.access_token.is_none() {
        return OAuthGrantState::Absent;
    }
    if !is_oauth_credential_expired(credential, now, buffer_seconds) {
        return OAuthGrantState::Valid;
    }
    if credential.refresh_token.is_some() {
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
    let stored = text_field(row, "credential_b64").ok_or(CredentialError::MissingField {
        field: "credential_b64",
    })?;
    let (credential_type, expires_at, connected_at) = match decode_oauth_payload(cipher, stored)? {
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
pub async fn list_server_user_credentials<A: CredentialRows>(
    table: &A,
    cipher: &DbCipher,
    server_id: &str,
) -> Result<Vec<McpUserCredentialListItem>, CredentialError> {
    let rows = table
        .find_credentials(CredentialScope::Server(server_id))
        .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        items.push(server_user_credential_item(row, cipher)?);
    }
    Ok(items)
}

// LiteLLM's list_user_oauth_credentials: a row that does not decrypt is warned about and skipped, a row that is not OAuth2 is skipped, and the payload is tagged with its server_id
pub async fn list_user_oauth_credentials<A: CredentialRows>(
    table: &A,
    cipher: &DbCipher,
    user_id: &str,
) -> Result<Vec<OAuthCredentialPayload>, CredentialError> {
    let rows = table
        .find_credentials(CredentialScope::User(user_id))
        .await?;
    let mut payloads = Vec::with_capacity(rows.len());
    for row in &rows {
        let server_id = server_id_of(row)?;
        let stored = text_field(row, "credential_b64").ok_or(CredentialError::MissingField {
            field: "credential_b64",
        })?;
        let decoded = decode_user_credential(cipher, stored);
        if decoded.is_none() {
            warn_undecryptable_credential(user_id, &server_id);
        }
        let Some(mut payload) = parse_oauth_payload(decoded.as_ref())? else {
            continue;
        };
        payload.server_id = Some(server_id);
        payloads.push(payload);
    }
    Ok(payloads)
}
