use betterlite_db::mcp_user_credentials::store::{
    delete_user_credential, get_user_credential, get_user_oauth_credential, store_user_credential,
};
use betterlite_db::mcp_user_credentials::types::{
    CredentialError, OAuthCredentialPayload, StoreOAuthCredential as OAuthRequest,
};
use betterlite_db::mcp_user_credentials::{
    list_server_user_credentials, list_user_oauth_credentials as list_oauth_payloads,
    store_user_oauth_credential,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use chrono::DateTime;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::rows::RowStore;
use super::{
    DeleteCredential, GetCredential, GetOauthCredential, ListServerCredentials,
    ListUserOauthCredentials, StoreCredential, StoreOauthCredential,
};

fn cipher(salt: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) async fn get_credential(call: &GetCredential) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let read = get_user_credential(
        &store,
        &cipher(&call.salt_key),
        &call.user_id,
        &call.server_id,
    )
    .await;
    Ok(read.or_value_error()?.map_or(Value::Null, Value::String))
}

pub(super) async fn get_oauth_credential(call: &GetOauthCredential) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let read = get_user_oauth_credential(
        &store,
        &cipher(&call.salt_key),
        &call.user_id,
        &call.server_id,
    )
    .await;
    let Some(payload) = read.or_value_error()? else {
        return Ok(Value::Null);
    };
    serde_json::to_value(&payload).or_value_error()
}

pub(super) async fn store_credential(call: &StoreCredential) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let db_cipher = cipher(&call.salt_key);
    store_user_credential(
        &store,
        &db_cipher,
        &call.user_id,
        &call.server_id,
        &call.credential,
    )
    .await
    .or_value_error()?;
    stored_view(&store, &db_cipher, &call.user_id, &call.server_id, false).await
}

pub(super) async fn store_oauth_credential(call: &StoreOauthCredential) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let db_cipher = cipher(&call.salt_key);
    let request = OAuthRequest {
        user_id: &call.user_id,
        server_id: &call.server_id,
        access_token: &call.access_token,
        refresh_token: call.refresh_token.as_deref(),
        expires_in: call.expires_in,
        scopes: call.scopes.as_deref(),
        skip_byok_guard: call.skip_byok_guard,
        identity_binding_proof: call.identity_binding_proof.as_deref(),
    };
    let written = store_user_oauth_credential(&store, &db_cipher, &request).await;
    let refused = match written {
        Ok(()) => false,
        Err(CredentialError::RefusesByokOverwrite { .. }) => true,
        Err(other) => return Err(other).or_value_error(),
    };
    stored_view(&store, &db_cipher, &call.user_id, &call.server_id, refused).await
}

// what the row holds after a write, without the timestamps that differ between two runs
async fn stored_view(
    store: &RowStore,
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
    refused: bool,
) -> HelperResult {
    let oauth = get_user_oauth_credential(store, cipher, user_id, server_id)
        .await
        .or_value_error()?;
    let Some(payload) = oauth else {
        let byok = get_user_credential(store, cipher, user_id, server_id)
            .await
            .or_value_error()?;
        return Ok(json!({"refused": refused, "oauth": Value::Null, "byok": byok}));
    };
    Ok(json!({"refused": refused, "oauth": payload_summary(&payload)?, "byok": Value::Null}))
}

fn payload_summary(payload: &OAuthCredentialPayload) -> HelperResult {
    let expires_in_seconds = match (
        payload.expires_at.as_deref(),
        payload.connected_at.as_deref(),
    ) {
        (Some(expires), Some(connected)) => Some(seconds_between(connected, expires)?),
        _ => None,
    };
    Ok(json!({
        "type": payload.credential_type,
        "access_token": payload.access_token,
        "refresh_token": payload.refresh_token,
        "scopes": payload.scopes,
        "identity_binding_proof": payload.identity_binding_proof,
        "connected_at_present": payload.connected_at.is_some(),
        "expires_in_seconds": expires_in_seconds,
    }))
}

fn seconds_between(connected_at: &str, expires_at: &str) -> HelperResult {
    let connected = DateTime::parse_from_rfc3339(connected_at).or_value_error()?;
    let expires = DateTime::parse_from_rfc3339(expires_at).or_value_error()?;
    let lifetime = expires.signed_duration_since(connected).num_seconds();
    Ok(Value::from(lifetime))
}

pub(super) async fn delete_credential(call: &DeleteCredential) -> HelperResult {
    let store = RowStore::new(&call.rows);
    delete_user_credential(&store, &call.user_id, &call.server_id)
        .await
        .or_value_error()?;
    Ok(json!({"rows": store.pairs()}))
}

pub(super) async fn list_server_credentials(call: &ListServerCredentials) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let items = list_server_user_credentials(&store, &cipher(&call.salt_key), &call.server_id)
        .await
        .or_value_error()?;
    serde_json::to_value(&items).or_value_error()
}

pub(super) async fn list_user_oauth(call: &ListUserOauthCredentials) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let payloads = list_oauth_payloads(&store, &cipher(&call.salt_key), &call.user_id)
        .await
        .or_value_error()?;
    serde_json::to_value(&payloads).or_value_error()
}
