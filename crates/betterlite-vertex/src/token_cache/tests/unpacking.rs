#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rstest::rstest;
use secrecy::SecretString;

use super::super::record::{CacheRecord, CredentialSnapshot, unpack_cached_credentials};
use super::super::state::{TokenState, token_state};
use super::legacy_snapshot;

struct ProjectionCase {
    state: Option<TokenState>,
    expires_in_seconds: Option<u64>,
    token: Option<&'static str>,
    expired: Option<bool>,
    valid: Option<bool>,
}

#[rstest]
#[case::reported_state_wins(
    ProjectionCase {
        state: Some(TokenState::Invalid),
        expires_in_seconds: Some(3600),
        token: Some("token"),
        expired: Some(false),
        valid: Some(true),
    },
    TokenState::Invalid
)]
#[case::expiry_far_ahead(
    ProjectionCase {
        state: None,
        expires_in_seconds: Some(3600),
        token: Some("token"),
        expired: None,
        valid: None,
    },
    TokenState::Fresh
)]
#[case::expiry_inside_threshold(
    ProjectionCase {
        state: None,
        expires_in_seconds: Some(100),
        token: Some("token"),
        expired: None,
        valid: None,
    },
    TokenState::Stale
)]
#[case::expiry_exactly_threshold(
    ProjectionCase {
        state: None,
        expires_in_seconds: Some(225),
        token: Some("token"),
        expired: None,
        valid: None,
    },
    TokenState::Stale
)]
#[case::expiry_just_past_threshold(
    ProjectionCase {
        state: None,
        expires_in_seconds: Some(226),
        token: Some("token"),
        expired: None,
        valid: None,
    },
    TokenState::Fresh
)]
#[case::expiry_reached(
    ProjectionCase {
        state: None,
        expires_in_seconds: Some(0),
        token: Some("token"),
        expired: None,
        valid: None,
    },
    TokenState::Invalid
)]
#[case::missing_token_with_expiry(
    ProjectionCase {
        state: None,
        expires_in_seconds: Some(3600),
        token: None,
        expired: None,
        valid: None,
    },
    TokenState::Invalid
)]
#[case::expired_flag(
    ProjectionCase {
        state: None,
        expires_in_seconds: None,
        token: Some("token"),
        expired: Some(true),
        valid: Some(true),
    },
    TokenState::Invalid
)]
#[case::valid_flag(
    ProjectionCase {
        state: None,
        expires_in_seconds: None,
        token: Some("token"),
        expired: Some(false),
        valid: Some(true),
    },
    TokenState::Fresh
)]
#[case::expired_false_without_valid(
    ProjectionCase {
        state: None,
        expires_in_seconds: None,
        token: Some("token"),
        expired: Some(false),
        valid: None,
    },
    TokenState::Invalid
)]
#[case::missing_expired_beats_valid(
    ProjectionCase {
        state: None,
        expires_in_seconds: None,
        token: Some("token"),
        expired: None,
        valid: Some(true),
    },
    TokenState::Invalid
)]
#[case::no_projection(
    ProjectionCase {
        state: None,
        expires_in_seconds: None,
        token: Some("token"),
        expired: None,
        valid: None,
    },
    TokenState::Invalid
)]
fn token_state_projects_reported_expiry_and_flags(
    #[case] case: ProjectionCase,
    #[case] expected: TokenState,
) {
    let now = SystemTime::UNIX_EPOCH;
    let expires_at = case
        .expires_in_seconds
        .map(|seconds| now.checked_add(Duration::from_secs(seconds)).unwrap());
    let credential = CredentialSnapshot {
        token: case.token.map(SecretString::from),
        state: case.state,
        expires_at,
        expired: case.expired,
        valid: case.valid,
        quota_project_id: None,
        project_id: None,
    };
    assert_eq!(token_state(&credential, now), expected);
}

#[rstest]
#[case::quota_wins(Some("quota-project"), Some("own-project"), Some("quota-project"))]
#[case::project_fallback(None, Some("own-project"), Some("own-project"))]
#[case::absent(None, None, None)]
fn unpack_recovers_the_legacy_project(
    #[case] quota: Option<&str>,
    #[case] project: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let record = CacheRecord::Legacy {
        credential: Arc::new(legacy_snapshot(quota, project)),
    };
    let (credential, recovered) = unpack_cached_credentials(Some(&record));
    assert!(credential.is_some());
    assert_eq!(recovered, expected);
}

#[test]
fn unpack_absent_record_is_empty() {
    let (credential, project) = unpack_cached_credentials(None);
    assert!(credential.is_none());
    assert!(project.is_none());
}

#[test]
fn unpack_paired_keeps_the_cached_project() {
    let record = CacheRecord::Paired {
        credential: Arc::new(legacy_snapshot(Some("quota-project"), Some("own-project"))),
        project: Some("cached-project".to_owned()),
    };
    let (credential, project) = unpack_cached_credentials(Some(&record));
    assert!(credential.is_some());
    assert_eq!(project, Some("cached-project"));
}
