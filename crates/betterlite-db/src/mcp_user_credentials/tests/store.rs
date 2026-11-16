use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta};

use super::super::decode::decode_user_credential;
use super::super::rows::text_field;
use super::super::store::{
    delete_user_credential, get_user_credential, get_user_oauth_credential, guard_byok_overwrite,
    store_user_credential,
};
use super::super::store_user_oauth_credential;
use super::super::types::{
    CredentialError, CredentialType, OAuthCredentialPayload, StoreOAuthCredential,
};
use super::support::{FakeTable, block_on, cipher, credential_row, warnings_during};
use super::{ENCRYPTED_OAUTH, ENCRYPTED_SECRET, NOT_JSON, UNDECRYPTABLE};

fn oauth_request() -> StoreOAuthCredential<'static> {
    StoreOAuthCredential {
        user_id: "u1",
        server_id: "s1",
        access_token: "tok-abc",
        refresh_token: None,
        expires_in: None,
        scopes: None,
        skip_byok_guard: false,
        identity_binding_proof: None,
    }
}

fn stored_payload(table: &FakeTable) -> OAuthCredentialPayload {
    let read = block_on(get_user_oauth_credential(table, &cipher(), "u1", "s1"));
    read.expect("the read runs")
        .expect("the row holds an OAuth2 payload")
}

fn decoded_stored_text(table: &FakeTable) -> String {
    let stored = table.stored();
    let ciphertext = text_field(&stored[0], "credential_b64").expect("the row stores the column");
    decode_user_credential(&cipher(), ciphertext).expect("the row decrypts")
}

#[test]
fn store_user_credential_encrypts_the_value_and_reads_it_back() {
    let table = FakeTable::new(Vec::new());
    block_on(store_user_credential(
        &table,
        &cipher(),
        "u1",
        "s1",
        "secret-value",
    ))
    .expect("the store runs");
    let stored = table.stored();
    assert_eq!(stored.len(), 1);
    let ciphertext = text_field(&stored[0], "credential_b64").expect("the row stores the column");
    assert_ne!(ciphertext, "secret-value");
    let read = block_on(get_user_credential(&table, &cipher(), "u1", "s1"));
    assert_eq!(
        read.expect("the read runs").as_deref(),
        Some("secret-value")
    );
}

#[test]
fn get_user_credential_returns_none_without_a_row() {
    let table = FakeTable::new(Vec::new());
    let read = block_on(get_user_credential(&table, &cipher(), "u1", "s1"));
    assert_eq!(read.expect("the read runs"), None);
}

#[test]
fn get_user_credential_reads_a_lite_llm_ciphertext() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", ENCRYPTED_SECRET)]);
    let read = block_on(get_user_credential(&table, &cipher(), "u1", "s1"));
    assert_eq!(
        read.expect("the read runs").as_deref(),
        Some("secret-value")
    );
}

#[test]
fn get_user_credential_returns_none_for_an_undecryptable_row() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", UNDECRYPTABLE)]);
    let read = block_on(get_user_credential(&table, &cipher(), "u1", "s1"));
    assert_eq!(read.expect("the read runs"), None);
}

#[test]
fn get_user_credential_requires_the_credential_column() {
    let row = JsonData::Object(BTreeMap::from([(
        "user_id".to_owned(),
        JsonData::String("u1".to_owned()),
    )]));
    let table = FakeTable::new(vec![row]);
    let failed = block_on(get_user_credential(&table, &cipher(), "u1", "s1"))
        .expect_err("the row has no credential column");
    assert!(matches!(
        failed,
        CredentialError::MissingField {
            field: "credential_b64"
        }
    ));
}

#[test]
fn delete_user_credential_removes_only_the_given_pair() {
    let table = FakeTable::new(vec![
        credential_row("u1", "s1", ENCRYPTED_SECRET),
        credential_row("u1", "s2", ENCRYPTED_SECRET),
    ]);
    block_on(delete_user_credential(&table, "u1", "s1")).expect("the delete runs");
    let stored = table.stored();
    assert_eq!(stored.len(), 1);
    assert_eq!(text_field(&stored[0], "server_id"), Some("s2"));
}

#[test]
fn get_user_oauth_credential_reads_an_oauth_payload() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", ENCRYPTED_OAUTH)]);
    let payload = stored_payload(&table);
    assert_eq!(payload.credential_type, CredentialType::Oauth2);
    assert_eq!(payload.access_token.as_deref(), Some("tok-123"));
    assert_eq!(
        payload.scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
}

#[test]
fn get_user_oauth_credential_returns_none_for_a_byok_key() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", ENCRYPTED_SECRET)]);
    let read = block_on(get_user_oauth_credential(&table, &cipher(), "u1", "s1"));
    assert!(read.expect("the read runs").is_none());
}

#[test]
fn get_user_oauth_credential_warns_when_the_row_does_not_decrypt() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", UNDECRYPTABLE)]);
    let warnings = warnings_during(|| {
        let read = block_on(get_user_oauth_credential(&table, &cipher(), "u1", "s1"));
        assert!(read.expect("the read runs").is_none());
    });
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("user_id=u1"));
    assert!(warnings[0].contains("server_id=s1"));
    assert!(warnings[0].contains("re-authorize"));
}

#[test]
fn byok_guard_allows_a_missing_row() {
    let guarded = guard_byok_overwrite(&cipher(), None, "u1", "s1");
    assert!(guarded.is_ok());
}

#[test]
fn byok_guard_allows_an_existing_oauth_payload() {
    let row = credential_row("u1", "s1", ENCRYPTED_OAUTH);
    let guarded = guard_byok_overwrite(&cipher(), Some(&row), "u1", "s1");
    assert!(guarded.is_ok());
}

#[test]
fn byok_guard_refuses_a_readable_byok_key() {
    let row = credential_row("u1", "s1", ENCRYPTED_SECRET);
    let failed = guard_byok_overwrite(&cipher(), Some(&row), "u1", "s1")
        .expect_err("a readable key is refused");
    let CredentialError::RefusesByokOverwrite { user_id, server_id } = failed else {
        panic!("a readable key is refused as a BYOK overwrite");
    };
    assert_eq!(user_id, "u1");
    assert_eq!(server_id, "s1");
}

#[test]
fn byok_guard_refuses_readable_text_that_is_not_a_payload() {
    let row = credential_row("u1", "s1", NOT_JSON);
    let guarded = guard_byok_overwrite(&cipher(), Some(&row), "u1", "s1");
    assert!(guarded.is_err());
}

#[test]
fn byok_guard_replaces_an_undecryptable_row_with_a_warning() {
    let row = credential_row("u1", "s1", UNDECRYPTABLE);
    let warnings = warnings_during(|| {
        let guarded = guard_byok_overwrite(&cipher(), Some(&row), "u1", "s1");
        assert!(guarded.is_ok());
    });
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("replacing"));
}

#[test]
fn byok_guard_replaces_a_row_that_decodes_to_nothing() {
    let table = FakeTable::new(Vec::new());
    block_on(store_user_credential(&table, &cipher(), "u1", "s1", "")).expect("the store runs");
    let row = table.stored().remove(0);
    let warnings = warnings_during(|| {
        let guarded = guard_byok_overwrite(&cipher(), Some(&row), "u1", "s1");
        assert!(guarded.is_ok());
    });
    assert_eq!(warnings.len(), 1);
}

#[test]
fn byok_guard_requires_the_credential_column() {
    let row = JsonData::Object(BTreeMap::from([(
        "user_id".to_owned(),
        JsonData::String("u1".to_owned()),
    )]));
    let guarded = guard_byok_overwrite(&cipher(), Some(&row), "u1", "s1");
    assert!(guarded.is_err());
}

#[test]
fn oauth_store_writes_the_payload_lite_llm_reads_back() {
    let table = FakeTable::new(Vec::new());
    block_on(store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(),
    ))
    .expect("the store runs");
    let payload = stored_payload(&table);
    assert_eq!(payload.credential_type, CredentialType::Oauth2);
    assert_eq!(payload.access_token.as_deref(), Some("tok-abc"));
    assert_eq!(payload.refresh_token, None);
    assert_eq!(payload.expires_at, None);
    assert!(payload.connected_at.is_some());
    assert_eq!(payload.scopes, None);
    assert_eq!(payload.identity_binding_proof, None);
    assert_eq!(payload.server_id, None);
}

#[test]
fn oauth_store_keeps_the_refresh_token_when_given() {
    let table = FakeTable::new(Vec::new());
    let mut request = oauth_request();
    request.refresh_token = Some("ref-1");
    block_on(store_user_oauth_credential(&table, &cipher(), &request)).expect("the store runs");
    assert_eq!(
        stored_payload(&table).refresh_token.as_deref(),
        Some("ref-1")
    );
}

#[test]
fn oauth_store_computes_expires_at_from_expires_in() {
    let table = FakeTable::new(Vec::new());
    let mut request = oauth_request();
    request.expires_in = Some(3600);
    block_on(store_user_oauth_credential(&table, &cipher(), &request)).expect("the store runs");
    let payload = stored_payload(&table);
    let connected_at = payload.connected_at.expect("connected_at is written");
    let expires_at = payload.expires_at.expect("expires_at is written");
    let connected =
        DateTime::parse_from_rfc3339(&connected_at).expect("connected_at is a timestamp");
    let expires = DateTime::parse_from_rfc3339(&expires_at).expect("expires_at is a timestamp");
    let lifetime = expires.signed_duration_since(connected);
    assert_eq!(
        lifetime,
        TimeDelta::try_seconds(3600).expect("the lifetime fits")
    );
}

#[test]
fn oauth_store_keeps_the_scopes_when_given() {
    let table = FakeTable::new(Vec::new());
    let scopes = vec!["read".to_owned(), "write".to_owned()];
    let mut request = oauth_request();
    request.scopes = Some(scopes.as_slice());
    block_on(store_user_oauth_credential(&table, &cipher(), &request)).expect("the store runs");
    assert_eq!(
        stored_payload(&table).scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
}

#[test]
fn stores_identity_binding_proof_only_when_present() {
    let table = FakeTable::new(Vec::new());
    block_on(store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(),
    ))
    .expect("the store runs");
    let decoded = decoded_stored_text(&table);
    assert!(!decoded.contains("identity_binding_proof"));
    let mut request = oauth_request();
    request.identity_binding_proof = Some("proof");
    block_on(store_user_oauth_credential(&table, &cipher(), &request)).expect("the store runs");
    assert_eq!(
        stored_payload(&table).identity_binding_proof.as_deref(),
        Some("proof")
    );
    assert!(decoded_stored_text(&table).contains("identity_binding_proof"));
}

#[test]
fn stores_an_empty_identity_binding_proof_that_is_present() {
    let table = FakeTable::new(Vec::new());
    let mut request = oauth_request();
    request.identity_binding_proof = Some("");
    block_on(store_user_oauth_credential(&table, &cipher(), &request)).expect("the store runs");
    let decoded = decoded_stored_text(&table);
    assert!(decoded.contains("\"identity_binding_proof\":\"\""));
}

#[test]
fn oauth_store_refuses_an_expires_in_beyond_the_calendar_range() {
    let table = FakeTable::new(Vec::new());
    let mut request = oauth_request();
    request.expires_in = Some(i64::MAX);
    let failed = block_on(store_user_oauth_credential(&table, &cipher(), &request))
        .expect_err("the lifetime does not fit");
    assert!(matches!(
        failed,
        CredentialError::ExpiresInOutOfRange { seconds: i64::MAX }
    ));
    assert!(table.stored().is_empty());
}

#[test]
fn oauth_store_refuses_to_overwrite_a_readable_byok_key() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", ENCRYPTED_SECRET)]);
    let failed = block_on(store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(),
    ))
    .expect_err("the guard refuses");
    assert!(matches!(
        failed,
        CredentialError::RefusesByokOverwrite { .. }
    ));
    let stored = table.stored();
    assert_eq!(
        text_field(&stored[0], "credential_b64"),
        Some(ENCRYPTED_SECRET)
    );
}

#[test]
fn oauth_store_overwrites_a_readable_byok_key_when_the_guard_is_skipped() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", ENCRYPTED_SECRET)]);
    let mut request = oauth_request();
    request.skip_byok_guard = true;
    block_on(store_user_oauth_credential(&table, &cipher(), &request))
        .expect("the guard is skipped");
    assert_eq!(
        stored_payload(&table).access_token.as_deref(),
        Some("tok-abc")
    );
}

#[test]
fn oauth_store_replaces_an_earlier_payload() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", ENCRYPTED_OAUTH)]);
    block_on(store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(),
    ))
    .expect("the store runs");
    assert_eq!(
        stored_payload(&table).access_token.as_deref(),
        Some("tok-abc")
    );
}

#[test]
fn oauth_store_replaces_an_undecryptable_row_with_a_warning() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", UNDECRYPTABLE)]);
    let warnings = warnings_during(|| {
        block_on(store_user_oauth_credential(
            &table,
            &cipher(),
            &oauth_request(),
        ))
        .expect("the replacement stores");
    });
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("replacing"));
    assert_eq!(
        stored_payload(&table).access_token.as_deref(),
        Some("tok-abc")
    );
}
