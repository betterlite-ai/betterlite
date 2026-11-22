#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Mutex, MutexGuard, PoisonError};

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use secrecy::{ExposeSecret as _, SecretString};

use super::exchange_authorization_code;
use crate::byok_token_exchange::ports::{
    AuthCodeLookup, AuthCodeRecord, ByokCredentialSink, SinkError,
};
use crate::byok_token_exchange::session_jwt::SessionClaims;
use crate::byok_token_exchange::{TokenError, TokenForm};

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const NOW: f64 = 1_761_000_000.0;
const CODE: &str = "auth-code-1";
const MASTER: &str = "sk-master-key";

#[derive(Debug, Clone, PartialEq)]
enum CodeCall {
    Purge(f64),
    Get(String),
    Remove(String),
}

#[derive(Default)]
struct FakeCodesState {
    record: Option<AuthCodeRecord>,
    calls: Vec<CodeCall>,
}

struct FakeCodes {
    state: Mutex<FakeCodesState>,
}

impl FakeCodes {
    fn new(record: AuthCodeRecord) -> Self {
        let state = FakeCodesState {
            record: Some(record),
            calls: Vec::new(),
        };
        Self {
            state: Mutex::new(state),
        }
    }

    fn empty() -> Self {
        Self {
            state: Mutex::new(FakeCodesState::default()),
        }
    }

    fn calls(&self) -> Vec<CodeCall> {
        self.lock().calls.clone()
    }

    fn stored(&self) -> bool {
        self.lock().record.is_some()
    }

    fn lock(&self) -> MutexGuard<'_, FakeCodesState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl AuthCodeLookup for FakeCodes {
    fn purge_expired(&self, now: f64) {
        self.lock().calls.push(CodeCall::Purge(now));
    }

    fn get(&self, code: &str) -> Option<AuthCodeRecord> {
        let mut state = self.lock();
        state.calls.push(CodeCall::Get(code.to_owned()));
        state.record.clone()
    }

    fn remove(&self, code: &str) {
        let mut state = self.lock();
        state.calls.push(CodeCall::Remove(code.to_owned()));
        state.record = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SinkCall {
    Store {
        user_id: String,
        server_id: String,
        credential: String,
    },
    Invalidate {
        user_id: String,
        server_id: String,
    },
}

#[derive(Default)]
struct FakeSinkState {
    calls: Vec<SinkCall>,
    fail_store: bool,
    fail_invalidate: bool,
}

#[derive(Default)]
struct FakeSink {
    state: Mutex<FakeSinkState>,
}

impl FakeSink {
    fn failing_store() -> Self {
        let sink = Self::default();
        sink.lock().fail_store = true;
        sink
    }

    fn failing_invalidate() -> Self {
        let sink = Self::default();
        sink.lock().fail_invalidate = true;
        sink
    }

    fn calls(&self) -> Vec<SinkCall> {
        self.lock().calls.clone()
    }

    fn lock(&self) -> MutexGuard<'_, FakeSinkState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish(&self, call: SinkCall) -> impl Future<Output = Result<(), SinkError>> + Send {
        let failure = match call {
            SinkCall::Store { .. } => self.store_failure(),
            SinkCall::Invalidate { .. } => self.invalidate_failure(),
        };
        self.lock().calls.push(call);
        ready(failure.map_or(Ok(()), Err))
    }

    fn store_failure(&self) -> Option<SinkError> {
        self.lock().fail_store.then(|| SinkError::Store {
            message: "database unavailable".to_owned(),
        })
    }

    fn invalidate_failure(&self) -> Option<SinkError> {
        self.lock().fail_invalidate.then(|| SinkError::Invalidate {
            message: "peer publish failed".to_owned(),
        })
    }
}

impl ByokCredentialSink for FakeSink {
    fn store_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential: &SecretString,
    ) -> impl Future<Output = Result<(), SinkError>> + Send {
        self.finish(store_call(user_id, server_id, credential))
    }

    fn invalidate_cache(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), SinkError>> + Send {
        self.finish(invalidate_call(user_id, server_id))
    }
}

fn store_call(user_id: &str, server_id: &str, credential: &SecretString) -> SinkCall {
    SinkCall::Store {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
        credential: credential.expose_secret().to_owned(),
    }
}

fn invalidate_call(user_id: &str, server_id: &str) -> SinkCall {
    SinkCall::Invalidate {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn record() -> AuthCodeRecord {
    AuthCodeRecord {
        api_key: secret("sk-user"),
        server_id: "srv-1".to_owned(),
        code_challenge: CHALLENGE.to_owned(),
        redirect_uri: String::new(),
        client_id: String::new(),
        user_id: "user-1".to_owned(),
        expires_at: 1_761_000_300.0,
    }
}

fn form() -> TokenForm {
    TokenForm {
        grant_type: "authorization_code".to_owned(),
        code: CODE.to_owned(),
        redirect_uri: String::new(),
        code_verifier: VERIFIER.to_owned(),
        client_id: String::new(),
    }
}

fn master() -> SecretString {
    secret(MASTER)
}

#[tokio::test]
async fn an_unsupported_grant_type_is_refused_before_the_lookup() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::default();
    let changed = TokenForm {
        grant_type: "refresh_token".to_owned(),
        ..form()
    };
    let result =
        exchange_authorization_code(&changed, &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::UnsupportedGrantType)));
    assert_eq!(codes.calls(), [CodeCall::Purge(NOW)]);
    assert!(codes.stored());
    assert!(sink.calls().is_empty());
}

#[tokio::test]
async fn an_unknown_code_is_invalid_grant() {
    let codes = FakeCodes::empty();
    let sink = FakeSink::default();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::InvalidGrant)));
    assert_eq!(
        codes.calls(),
        [CodeCall::Purge(NOW), CodeCall::Get(CODE.to_owned())]
    );
    assert!(sink.calls().is_empty());
}

#[tokio::test]
async fn an_expired_code_is_removed_and_refused() {
    let codes = FakeCodes::new(AuthCodeRecord {
        expires_at: 1_760_999_999.0,
        ..record()
    });
    let sink = FakeSink::default();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::InvalidGrant)));
    assert_eq!(
        codes.calls(),
        [
            CodeCall::Purge(NOW),
            CodeCall::Get(CODE.to_owned()),
            CodeCall::Remove(CODE.to_owned()),
        ]
    );
    assert!(!codes.stored());
    assert!(sink.calls().is_empty());
}

#[tokio::test]
async fn a_code_expiring_exactly_now_is_still_exchanged() {
    let codes = FakeCodes::new(AuthCodeRecord {
        expires_at: NOW,
        ..record()
    });
    let sink = FakeSink::default();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    let grant = result.unwrap();
    assert_eq!(grant.expires_in, 3600);
    assert!(!codes.stored());
}

#[tokio::test]
async fn a_wrong_verifier_keeps_the_code() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::default();
    let changed = TokenForm {
        code_verifier: "not-the-verifier".to_owned(),
        ..form()
    };
    let result =
        exchange_authorization_code(&changed, &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::InvalidGrant)));
    assert_eq!(
        codes.calls(),
        [CodeCall::Purge(NOW), CodeCall::Get(CODE.to_owned())]
    );
    assert!(codes.stored());
    assert!(sink.calls().is_empty());
}

#[tokio::test]
async fn a_mismatched_redirect_uri_keeps_the_code() {
    let codes = FakeCodes::new(AuthCodeRecord {
        redirect_uri: "https://app.test/callback".to_owned(),
        ..record()
    });
    let sink = FakeSink::default();
    let changed = TokenForm {
        redirect_uri: "https://app.test/other".to_owned(),
        ..form()
    };
    let result =
        exchange_authorization_code(&changed, &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::InvalidGrant)));
    assert!(codes.stored());
}

#[tokio::test]
async fn a_missing_redirect_uri_passes() {
    let codes = FakeCodes::new(AuthCodeRecord {
        redirect_uri: "https://app.test/callback".to_owned(),
        ..record()
    });
    let sink = FakeSink::default();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    assert_eq!(result.unwrap().token_type, "bearer");
}

#[tokio::test]
async fn an_empty_stored_redirect_uri_skips_the_check() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::default();
    let changed = TokenForm {
        redirect_uri: "https://app.test/anything".to_owned(),
        ..form()
    };
    let result =
        exchange_authorization_code(&changed, &codes, Some(&sink), Some(&master()), NOW).await;
    assert_eq!(result.unwrap().token_type, "bearer");
}

#[tokio::test]
async fn a_different_client_id_keeps_the_code() {
    let codes = FakeCodes::new(AuthCodeRecord {
        client_id: "client-1".to_owned(),
        ..record()
    });
    let sink = FakeSink::default();
    let changed = TokenForm {
        client_id: "client-2".to_owned(),
        ..form()
    };
    let result =
        exchange_authorization_code(&changed, &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::InvalidGrant)));
    assert!(codes.stored());
}

#[tokio::test]
async fn an_empty_client_id_does_not_match_a_stored_one() {
    let codes = FakeCodes::new(AuthCodeRecord {
        client_id: "client-1".to_owned(),
        ..record()
    });
    let sink = FakeSink::default();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::InvalidGrant)));
    assert!(codes.stored());
}

#[tokio::test]
async fn a_matching_client_id_passes() {
    let codes = FakeCodes::new(AuthCodeRecord {
        client_id: "client-1".to_owned(),
        ..record()
    });
    let sink = FakeSink::default();
    let changed = TokenForm {
        client_id: "client-1".to_owned(),
        ..form()
    };
    let result =
        exchange_authorization_code(&changed, &codes, Some(&sink), Some(&master()), NOW).await;
    assert_eq!(result.unwrap().token_type, "bearer");
}

#[tokio::test]
async fn an_empty_stored_client_id_skips_the_check() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::default();
    let changed = TokenForm {
        client_id: "anything".to_owned(),
        ..form()
    };
    let result =
        exchange_authorization_code(&changed, &codes, Some(&sink), Some(&master()), NOW).await;
    assert_eq!(result.unwrap().token_type, "bearer");
}

#[tokio::test]
async fn an_empty_user_id_keeps_the_code() {
    let codes = FakeCodes::new(AuthCodeRecord {
        user_id: String::new(),
        ..record()
    });
    let sink = FakeSink::default();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::InvalidGrant)));
    assert!(codes.stored());
}

#[tokio::test]
async fn a_missing_master_key_fails_before_the_code_is_removed() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::default();
    let result = exchange_authorization_code(&form(), &codes, Some(&sink), None, NOW).await;
    assert!(matches!(result, Err(TokenError::ServerError)));
    assert_eq!(
        codes.calls(),
        [CodeCall::Purge(NOW), CodeCall::Get(CODE.to_owned())]
    );
    assert!(codes.stored());
    assert!(sink.calls().is_empty());
}

#[tokio::test]
async fn a_successful_exchange_stores_then_invalidates_and_returns_a_bearer_grant() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::default();
    let grant = exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW)
        .await
        .unwrap();
    assert_eq!(grant.token_type, "bearer");
    assert_eq!(grant.expires_in, 3600);
    assert!(!grant.access_token.is_empty());
    assert_eq!(
        codes.calls(),
        [
            CodeCall::Purge(NOW),
            CodeCall::Get(CODE.to_owned()),
            CodeCall::Remove(CODE.to_owned()),
        ]
    );
    assert_eq!(
        sink.calls(),
        [
            SinkCall::Store {
                user_id: "user-1".to_owned(),
                server_id: "srv-1".to_owned(),
                credential: "sk-user".to_owned(),
            },
            SinkCall::Invalidate {
                user_id: "user-1".to_owned(),
                server_id: "srv-1".to_owned(),
            },
        ]
    );
}

#[tokio::test]
async fn a_failed_store_removes_the_code_and_skips_invalidation() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::failing_store();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::ServerError)));
    assert!(!codes.stored());
    assert_eq!(
        sink.calls(),
        [SinkCall::Store {
            user_id: "user-1".to_owned(),
            server_id: "srv-1".to_owned(),
            credential: "sk-user".to_owned(),
        }]
    );
}

#[tokio::test]
async fn a_failed_invalidation_keeps_the_written_credential() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::failing_invalidate();
    let result =
        exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW).await;
    assert!(matches!(result, Err(TokenError::ServerError)));
    assert!(!codes.stored());
    assert_eq!(
        sink.calls(),
        [
            SinkCall::Store {
                user_id: "user-1".to_owned(),
                server_id: "srv-1".to_owned(),
                credential: "sk-user".to_owned(),
            },
            SinkCall::Invalidate {
                user_id: "user-1".to_owned(),
                server_id: "srv-1".to_owned(),
            },
        ]
    );
}

#[tokio::test]
async fn a_missing_sink_still_returns_a_grant() {
    let codes = FakeCodes::new(record());
    let result =
        exchange_authorization_code(&form(), &codes, None::<&FakeSink>, Some(&master()), NOW).await;
    assert!(!result.unwrap().access_token.is_empty());
    assert!(!codes.stored());
}

#[tokio::test]
async fn the_session_token_is_signed_with_the_master_key() {
    let codes = FakeCodes::new(record());
    let sink = FakeSink::default();
    let grant = exchange_authorization_code(&form(), &codes, Some(&sink), Some(&master()), NOW)
        .await
        .unwrap();
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    let key = DecodingKey::from_secret(master().expose_secret().as_bytes());
    let decoded = decode::<SessionClaims>(&grant.access_token, &key, &validation).unwrap();
    assert_eq!(decoded.claims.user_id, "user-1");
    assert_eq!(decoded.claims.server_id, "srv-1");
    assert_eq!(decoded.claims.iat, 1_761_000_000);
    assert_eq!(decoded.claims.exp, 1_761_003_600);
    let other = DecodingKey::from_secret(b"sk-other-key");
    let wrong_key = decode::<SessionClaims>(&grant.access_token, &other, &validation).unwrap_err();
    assert!(matches!(wrong_key.kind(), ErrorKind::InvalidSignature));
}
