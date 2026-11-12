#![cfg(test)]

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::key::CredentialKey;
use super::super::probe::{try_cached_token, try_usable_cached_token};
use super::super::record::{CacheRecord, CredentialSnapshot};
use super::super::state::TokenState;
use super::legacy_snapshot;

struct ProbeFixture {
    token: Option<&'static str>,
    state: Option<TokenState>,
    cached_project: Option<&'static str>,
    legacy: bool,
    requested: Option<&'static str>,
    expected_fresh: Option<(&'static str, &'static str)>,
    expected_usable: Option<(&'static str, &'static str, TokenState)>,
}

#[rstest]
#[case::fresh_requested(ProbeFixture { token: Some("token-fresh"), state: Some(TokenState::Fresh), cached_project: Some("cached-project"), legacy: false, requested: Some("explicit-project"), expected_fresh: Some(("token-fresh", "explicit-project")), expected_usable: Some(("token-fresh", "explicit-project", TokenState::Fresh)) })]
#[case::fresh_cached(ProbeFixture { token: Some("token-fresh"), state: Some(TokenState::Fresh), cached_project: Some("cached-project"), legacy: false, requested: None, expected_fresh: Some(("token-fresh", "cached-project")), expected_usable: Some(("token-fresh", "cached-project", TokenState::Fresh)) })]
#[case::legacy_project(ProbeFixture { token: Some("token-legacy"), state: Some(TokenState::Fresh), cached_project: Some("ignored-project"), legacy: true, requested: None, expected_fresh: Some(("token-legacy", "own-project")), expected_usable: Some(("token-legacy", "own-project", TokenState::Fresh)) })]
#[case::stale_token(ProbeFixture { token: Some("token-stale"), state: Some(TokenState::Stale), cached_project: Some("cached-project"), legacy: false, requested: None, expected_fresh: None, expected_usable: Some(("token-stale", "cached-project", TokenState::Stale)) })]
#[case::invalid_state(ProbeFixture { token: Some("token-invalid"), state: Some(TokenState::Invalid), cached_project: Some("cached-project"), legacy: false, requested: Some("explicit-project"), expected_fresh: None, expected_usable: None })]
#[case::missing_token(ProbeFixture { token: None, state: Some(TokenState::Fresh), cached_project: Some("cached-project"), legacy: false, requested: Some("explicit-project"), expected_fresh: None, expected_usable: None })]
#[case::unresolved_project(ProbeFixture { token: Some("token-fresh"), state: Some(TokenState::Fresh), cached_project: None, legacy: false, requested: None, expected_fresh: None, expected_usable: None })]
fn probes_project_fresh_stale_and_invalid_cases(#[case] fixture: ProbeFixture) {
    let key = CredentialKey {
        credential: Some(SecretString::from("credential-key")),
        project: Some("key-project".to_owned()),
    };
    let credential = Arc::new(CredentialSnapshot {
        token: fixture.token.map(SecretString::from),
        state: fixture.state,
        expires_at: None,
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: Some("own-project".to_owned()),
    });
    let record = if fixture.legacy {
        CacheRecord::Legacy { credential }
    } else {
        CacheRecord::Paired {
            credential,
            project: fixture.cached_project.map(str::to_owned),
        }
    };
    let mut records = BTreeMap::new();
    records.insert(key.clone(), record);
    let fresh = try_cached_token(&records, &key, fixture.requested, SystemTime::UNIX_EPOCH);
    assert_eq!(
        fresh.map(|access| (access.token.expose_secret().to_owned(), access.project)),
        fixture
            .expected_fresh
            .map(|(token, project)| (token.to_owned(), project.to_owned()))
    );
    let expected_cached = if fixture.legacy {
        Some("own-project")
    } else {
        fixture.cached_project
    };
    let usable = try_usable_cached_token(&records, &key, fixture.requested, SystemTime::UNIX_EPOCH);
    assert_eq!(
        usable.map(|entry| (
            entry.token.expose_secret().to_owned(),
            entry.project,
            entry.state,
            entry.cached_project,
        )),
        fixture.expected_usable.map(|(token, project, state)| (
            token.to_owned(),
            project.to_owned(),
            state,
            expected_cached.map(str::to_owned),
        ))
    );
}

#[test]
fn probes_of_an_absent_record_are_empty() {
    let records: BTreeMap<CredentialKey, CacheRecord> = BTreeMap::new();
    let key = CredentialKey {
        credential: Some(SecretString::from("credential-key")),
        project: None,
    };
    assert!(try_cached_token(&records, &key, None, SystemTime::UNIX_EPOCH).is_none());
    assert!(try_usable_cached_token(&records, &key, None, SystemTime::UNIX_EPOCH).is_none());
}

#[test]
fn a_record_hands_out_its_credential() {
    let snapshot = Arc::new(legacy_snapshot(Some("quota-project"), None));
    let paired = CacheRecord::Paired {
        credential: Arc::clone(&snapshot),
        project: None,
    };
    let legacy = CacheRecord::Legacy {
        credential: Arc::clone(&snapshot),
    };
    assert!(Arc::ptr_eq(paired.credential(), &snapshot));
    assert!(Arc::ptr_eq(legacy.credential(), &snapshot));
}

#[test]
fn credential_keys_order_by_project_then_credential() {
    let empty = CredentialKey {
        credential: None,
        project: None,
    };
    let credential_only = CredentialKey {
        credential: Some(SecretString::from("credential")),
        project: None,
    };
    let with_project = CredentialKey {
        credential: Some(SecretString::from("credential")),
        project: Some("project".to_owned()),
    };
    assert_eq!(empty.cmp(&credential_only), Ordering::Less);
    assert_eq!(credential_only.cmp(&empty), Ordering::Greater);
    assert_eq!(empty.cmp(&empty), Ordering::Equal);
    assert_eq!(with_project.cmp(&credential_only), Ordering::Greater);
    assert_eq!(empty.partial_cmp(&credential_only), Some(Ordering::Less));
    assert_eq!(
        with_project.partial_cmp(&credential_only),
        Some(Ordering::Greater)
    );
}
