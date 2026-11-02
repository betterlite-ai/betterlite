#![cfg(test)]

use super::AuthError;

#[test]
fn no_database_has_no_http_mapping() {
    let error = AuthError::NoDatabaseConnection;
    assert_eq!(error.kind(), "no_database_connection");
    assert!(error.http_mapping().is_none());
}

#[test]
fn cache_only_miss_is_distinct() {
    let error = AuthError::KeyNotInCache {
        hashed_token: "deadbeef".to_owned(),
    };
    assert_eq!(error.kind(), "key_not_in_cache");
    assert_ne!(
        error.kind(),
        AuthError::KeyNotFound {
            hashed_token: "deadbeef".to_owned()
        }
        .kind()
    );
    assert!(error.http_mapping().is_none());
}

#[test]
fn missing_key_401_token_not_found() {
    let error = AuthError::KeyNotFound {
        hashed_token: "deadbeef".to_owned(),
    };
    let mapping = error.http_mapping().unwrap();
    assert_eq!(mapping.status, 401);
    assert_eq!(mapping.error_type, "token_not_found_in_db");
    assert_eq!(mapping.param, "key");
}

#[test]
fn missing_key_empty_headers_and_string_code() {
    let error = AuthError::KeyNotFound {
        hashed_token: "deadbeef".to_owned(),
    };
    let mapping = error.http_mapping().unwrap();
    assert_eq!(mapping.code, "401");
    assert!(mapping.headers.is_empty());
}

#[test]
fn missing_source_key_is_distinct() {
    let error = AuthError::PrincipalMissingSourceKey;
    assert_eq!(error.kind(), "principal_missing_source_key");
    assert!(error.http_mapping().is_none());
}

#[test]
fn absent_user_not_database_failure() {
    let error = AuthError::UserNotFound {
        user_id: "u1".to_owned(),
    };
    assert_eq!(error.kind(), "user_not_found");
    assert_ne!(error.kind(), AuthError::NoDatabaseConnection.kind());
    assert!(error.http_mapping().is_none());
}
