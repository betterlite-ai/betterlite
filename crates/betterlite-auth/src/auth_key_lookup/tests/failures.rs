#![cfg(test)]

use betterlite_db::combined_key_read::error::KeyReadError;
use betterlite_db::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use betterlite_db::database_faults::ErrorType;
use rstest::rstest;

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_key_lookup::error::KeyLookupError;
use crate::auth_prefetch::permission::lookup::PermissionError;

#[rstest]
#[case::missing_token(KeyLookupError::Read(KeyReadError::MissingToken), "auth_error", "400")]
#[case::permission(
    KeyLookupError::Permission(PermissionError::MissingInDatabaseOnly),
    "auth_error",
    "403"
)]
#[case::no_database(KeyLookupError::NoDatabase, "auth_error", "401")]
#[case::cache_only(KeyLookupError::NotInCache { key: "hash".to_owned() }, "auth_error", "401")]
fn public_failures(#[case] error: KeyLookupError, #[case] kind: &str, #[case] status: &str) {
    let failure = error.failure();
    let converted = as_proxy_exception(&failure);
    assert_eq!(converted.exception().error_type, kind);
    assert_eq!(converted.exception().code, status);
    assert!(converted.exception().message.contains(&error.to_string()));
}

#[test]
fn deadline_keeps_database_class() {
    let error = KeyLookupError::Lookup(DbLookupError::Deadline(DbLookupDeadlineExceeded {
        lookup: "key".to_owned(),
        deadline_seconds: 2.0,
        cause: None,
    }));
    let AuthFailure::Database(fault) = error.failure() else {
        panic!("deadline is a database fault");
    };
    assert_eq!(fault.error_type, ErrorType::Deadline);
    assert!(fault.message.contains("key"));
}
