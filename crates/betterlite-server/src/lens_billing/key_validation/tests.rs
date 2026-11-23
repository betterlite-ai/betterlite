#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_identity::errors::AuthError;
use rstest::rstest;

use super::{KeyResolver, KeyValidationError, LensKey, validate_key};

struct FakeResolver {
    calls: Mutex<Vec<String>>,
    answer: Mutex<Option<Result<LensKey, AuthError>>>,
}

impl FakeResolver {
    fn answering(answer: Result<LensKey, AuthError>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answer: Mutex::new(Some(answer)),
        }
    }

    fn recorded(&self) -> Vec<String> {
        self.calls.lock().expect("the resolver call log").clone()
    }
}

#[async_trait]
impl KeyResolver for FakeResolver {
    async fn resolve(&self, hashed_token: &str) -> Result<LensKey, AuthError> {
        self.calls
            .lock()
            .expect("the resolver call log")
            .push(hashed_token.to_owned());
        self.answer
            .lock()
            .expect("the resolver answer")
            .take()
            .expect("the test resolves one key")
    }
}

#[tokio::test]
async fn validate_key_without_a_key_returns_none_without_resolving() {
    let resolver = FakeResolver::answering(Err(AuthError::NoDatabaseConnection));
    let key = validate_key(&resolver, None).await.unwrap();
    assert_eq!(key, None);
    assert!(resolver.recorded().is_empty());
}

#[rstest]
#[case::blocked(true, false)]
#[case::session_token(false, true)]
#[case::both(true, true)]
#[tokio::test]
async fn validate_key_rejects_an_inactive_key(
    #[case] blocked: bool,
    #[case] is_session_token: bool,
) {
    let resolver = FakeResolver::answering(Ok(LensKey {
        blocked,
        is_session_token,
        team_id: Some("team-a".to_owned()),
    }));
    let error = validate_key(&resolver, Some("hashed-token"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyValidationError::Inactive));
    assert_eq!(error.status(), 400);
    assert_eq!(
        error.detail(),
        "Choose an active virtual key for Lens analysis"
    );
}

#[rstest]
#[case::with_team(Some("team-a"))]
#[case::without_team(None)]
#[tokio::test]
async fn validate_key_returns_an_active_key_with_its_team(#[case] team_id: Option<&str>) {
    let resolver = FakeResolver::answering(Ok(LensKey {
        blocked: false,
        is_session_token: false,
        team_id: team_id.map(str::to_owned),
    }));
    let key = validate_key(&resolver, Some("hashed-token"))
        .await
        .unwrap()
        .unwrap();
    assert!(!key.blocked);
    assert!(!key.is_session_token);
    assert_eq!(key.team_id.as_deref(), team_id);
}

#[tokio::test]
async fn validate_key_passes_the_key_id_to_the_resolver_byte_for_byte() {
    let key_id = "AbCdEf0123456789aBcDeF-session-Token";
    let resolver = FakeResolver::answering(Ok(LensKey {
        blocked: false,
        is_session_token: false,
        team_id: None,
    }));
    let key = validate_key(&resolver, Some(key_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key.team_id, None);
    assert_eq!(resolver.recorded(), vec![key_id.to_owned()]);
}

#[tokio::test]
async fn validate_key_treats_an_empty_key_id_as_present() {
    let resolver = FakeResolver::answering(Err(AuthError::KeyNotFound {
        hashed_token: String::new(),
    }));
    let error = validate_key(&resolver, Some("")).await.unwrap_err();
    assert_eq!(error.status(), 401);
    assert_eq!(resolver.recorded(), vec![String::new()]);
}

#[rstest]
#[case::key_not_found(
    AuthError::KeyNotFound {
        hashed_token: "hashed-token".to_owned(),
    },
    401
)]
#[case::no_database(AuthError::NoDatabaseConnection, 500)]
#[case::cache_only_miss(
    AuthError::KeyNotInCache {
        hashed_token: "hashed-token".to_owned(),
    },
    500
)]
#[case::principal_without_source_key(AuthError::PrincipalMissingSourceKey, 500)]
#[case::absent_user(
    AuthError::UserNotFound {
        user_id: "user-1".to_owned(),
    },
    500
)]
#[tokio::test]
async fn validate_key_passes_resolver_errors_through(
    #[case] failure: AuthError,
    #[case] status: u16,
) {
    let expected_detail = failure.to_string();
    let resolver = FakeResolver::answering(Err(failure));
    let error = validate_key(&resolver, Some("hashed-token"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyValidationError::Resolve(_)));
    assert_eq!(error.status(), status);
    assert_eq!(error.detail(), expected_detail);
}
