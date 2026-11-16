use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, TimeDelta, Utc};

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::rows::server_id_of;
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

// ast-grep-ignore: no-long-names
pub const MCP_PER_USER_TOKEN_EXPIRY_BUFFER_SECONDS: i64 = 60;

const NAIVE_FORMATS: [&str; 4] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
];

const OFFSET_FORMATS: [&str; 2] = ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%d %H:%M:%S%.f%z"];

// LiteLLM's store_user_oauth_credential: one JSON payload encrypted into the BYOK column, with a fresh connected_at and expires_at from the same instant
pub async fn store_user_oauth_credential<A: CredentialRows>(
    table: &A,
    cipher: &DbCipher,
    request: &StoreOAuthCredential<'_>,
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
    let payload = oauth_payload(request, Utc::now())?;
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
        access_token: Some(request.access_token.to_owned()),
        credential_type: CredentialType::Oauth2,
        refresh_token: request.refresh_token.map(str::to_owned),
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

// Python's isoformat writes six fractional digits only when the microsecond is set, and +00:00 for UTC
fn isoformat(moment: DateTime<Utc>) -> String {
    let precision = if moment.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    moment.to_rfc3339_opts(precision, false)
}

// Python's fromisoformat reads a bare Z, a space separator, a naive value and a date alone; the forms it also reads and this does not are in docs/deviations
fn parse_iso_moment(value: &str) -> Option<DateTime<Utc>> {
    let normalized = value.replace('Z', "+00:00");
    if let Ok(moment) = DateTime::parse_from_rfc3339(&normalized) {
        return Some(moment.with_timezone(&Utc));
    }
    for format in OFFSET_FORMATS {
        if let Ok(moment) = DateTime::parse_from_str(&normalized, format) {
            return Some(moment.with_timezone(&Utc));
        }
    }
    for format in NAIVE_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, format) {
            return Some(naive.and_utc());
        }
    }
    let midnight = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0));
    midnight.map(NaiveDateTime::and_utc)
}

// LiteLLM's is_oauth_credential_expired: a naive timestamp is UTC, and an absent or unparseable one never counts as expired
#[must_use]
pub fn is_oauth_credential_expired(
    credential: &OAuthCredentialPayload,
    buffer_seconds: i64,
) -> bool {
    let Some(expires_at) = credential.expires_at.as_deref() else {
        return false;
    };
    let Some(expires) = parse_iso_moment(expires_at) else {
        return false;
    };
    let Some(buffer) = TimeDelta::try_seconds(buffer_seconds) else {
        return false;
    };
    let now = Utc::now();
    now.checked_add_signed(buffer)
        .is_some_and(|deadline| deadline > expires)
}

// LiteLLM's oauth_grant_state: a token inside the refresh buffer is not valid, and only a stored refresh token makes it refreshable
#[must_use]
pub fn oauth_grant_state(credential: Option<&OAuthCredentialPayload>) -> OAuthGrantState {
    let Some(credential) = credential else {
        return OAuthGrantState::Absent;
    };
    if credential.access_token.is_none() {
        return OAuthGrantState::Absent;
    }
    if !is_oauth_credential_expired(credential, MCP_PER_USER_TOKEN_EXPIRY_BUFFER_SECONDS) {
        return OAuthGrantState::Valid;
    }
    if credential.refresh_token.is_some() {
        return OAuthGrantState::Refreshable;
    }
    OAuthGrantState::Absent
}

// LiteLLM's _server_user_credential_item: a row whose payload is not OAuth2 is a BYOK key, and neither shape carries the secret
#[must_use]
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
    let (credential_type, expires_at, connected_at) = decode_oauth_payload(cipher, stored).map_or(
        (CredentialType::Byok, None, None),
        |payload| {
            (
                CredentialType::Oauth2,
                payload.expires_at,
                payload.connected_at,
            )
        },
    );
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

// LiteLLM's list_user_oauth_credentials: a row that does not decrypt is warned about and skipped, and the payload is tagged with its server_id
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
        let Some(mut payload) = parse_oauth_payload(decoded.as_deref()).ok().flatten() else {
            continue;
        };
        payload.server_id = Some(server_id);
        payloads.push(payload);
    }
    Ok(payloads)
}
