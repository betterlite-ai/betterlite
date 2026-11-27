#![cfg(test)]

use std::io::Error as IoError;

use betterlite_auth::auth_identity::errors::AuthError as IdentityError;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_user_lookup::error::{UserLookupError, UserReadFailure};
use betterlite_auth::auth_user_lookup::ports::PortError;
use betterlite_auth::auth_user_lookup::rows::UserRowError;
use betterlite_auth::identity_store::error::IdentityStoreError;
use betterlite_auth::resolved_key::auth::AuthError as CredentialError;
use betterlite_db::combined_key_read::error::{DatabaseFailure, KeyReadError, ReconnectFailure};
use betterlite_db::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::postgres_store::errors::DbError as NativeDbError;

use super::super::{IdentityLookup, SubjectReadError};
use super::errors::{identity_error, key_read_error, user_read_error};

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct PlainError {
    message: String,
}

fn fault(error_type: ErrorType, message: &str) -> DbError {
    DbError {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

fn outage() -> DbError {
    fault(ErrorType::Connect, "connection refused")
}

fn permanent_fault() -> DbError {
    fault(ErrorType::Prisma, "the query engine reported a fault")
}

fn data_fault() -> DbError {
    fault(ErrorType::Data, "invalid input syntax")
}

fn data_with_cause(cause: DbError) -> DbError {
    let mut root = data_fault();
    root.cause = Some(Box::new(cause));
    root
}

fn deadline(cause: Option<DbLookupError>) -> DbLookupError {
    DbLookupError::Deadline(DbLookupDeadlineExceeded {
        lookup: "user_id:user-1".to_owned(),
        deadline_seconds: 5.0,
        cause: cause.map(Box::new),
    })
}

fn plain(message: &str) -> PlainError {
    PlainError {
        message: message.to_owned(),
    }
}

fn missing_user() -> UserLookupError {
    UserLookupError::NotFound(IdentityError::UserNotFound {
        user_id: "user-1".to_owned(),
    })
}

#[test]
fn key_missing_row_is_refused() {
    let error = KeyLookupError::MissingKey {
        key: "hashed-token".to_owned(),
    };
    assert_eq!(key_read_error(&error), SubjectReadError::Refused);
}

#[test]
fn key_missing_token_is_refused() {
    let error = KeyLookupError::Read(KeyReadError::MissingToken);
    assert_eq!(key_read_error(&error), SubjectReadError::Refused);
}

#[test]
fn key_direct_outage_is_unavailable() {
    let error = KeyLookupError::Lookup(DbLookupError::Database(outage()));
    assert_eq!(key_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn key_permanent_fault_is_faulted() {
    let error = KeyLookupError::Lookup(DbLookupError::Database(permanent_fault()));
    assert_eq!(key_read_error(&error), SubjectReadError::Faulted);
}

#[test]
fn key_data_fault_is_other() {
    let error = KeyLookupError::Lookup(DbLookupError::Database(data_fault()));
    assert_eq!(key_read_error(&error), SubjectReadError::Other);
}

#[test]
fn key_wrapped_outage_stays_other() {
    let error = KeyLookupError::Read(KeyReadError::Database(DatabaseFailure::Reported(
        data_with_cause(outage()),
    )));
    assert_eq!(key_read_error(&error), SubjectReadError::Other);
}

#[test]
fn key_deadline_is_unavailable() {
    let error = KeyLookupError::Lookup(deadline(None));
    assert_eq!(key_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn key_deadline_with_a_permanent_cause_is_faulted() {
    let cause = DbLookupError::Database(permanent_fault());
    let error = KeyLookupError::Lookup(deadline(Some(cause)));
    assert_eq!(key_read_error(&error), SubjectReadError::Faulted);
}

#[test]
fn key_deadline_with_a_transient_cause_is_unavailable() {
    let cause = DbLookupError::Database(outage());
    let error = KeyLookupError::Lookup(deadline(Some(cause)));
    assert_eq!(key_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn key_non_database_failure_is_other() {
    let error = KeyLookupError::Read(KeyReadError::Reconnect(ReconnectFailure {
        message: "the reconnect failed".to_owned(),
    }));
    assert_eq!(key_read_error(&error), SubjectReadError::Other);
}

#[test]
fn user_missing_row_is_other() {
    assert_eq!(user_read_error(&missing_user()), SubjectReadError::Other);
}

#[test]
fn user_without_a_database_is_other() {
    assert_eq!(
        user_read_error(&UserLookupError::NoDatabase),
        SubjectReadError::Other
    );
}

#[test]
fn user_direct_outage_is_unavailable() {
    let error = UserLookupError::Unavailable(Box::new(outage()));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_permanent_fault_is_faulted() {
    let error = UserLookupError::Unavailable(Box::new(permanent_fault()));
    assert_eq!(user_read_error(&error), SubjectReadError::Faulted);
}

#[test]
fn user_permanent_fault_outranks_a_transient_one() {
    let error = UserLookupError::Unavailable(Box::new(data_with_cause(permanent_fault())));
    assert_eq!(user_read_error(&error), SubjectReadError::Faulted);
}

#[test]
fn user_wrapped_outage_is_unavailable() {
    let read = AuthReadError::<UserRowError>::Lookup(DbLookupError::Database(outage()));
    let error = UserLookupError::Unavailable(Box::new(read));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_repository_outage_is_unavailable() {
    let read = AuthReadError::<UserRowError>::Repository(NativeDbError::NoDbConnected);
    let error = UserLookupError::Unavailable(Box::new(read));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_native_outage_is_unavailable() {
    let error =
        UserLookupError::Unavailable(Box::new(NativeDbError::<UserRowError>::NoDbConnected));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_port_fault_is_unavailable() {
    let port = PortError {
        message: "the default team enrollment failed".to_owned(),
        cause: None,
        database_fault: Some(Box::new(outage())),
    };
    let error = UserLookupError::Unavailable(Box::new(port));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_io_error_is_unavailable() {
    let error = UserLookupError::Unavailable(Box::new(IoError::other("the socket closed")));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_stale_plan_message_is_unavailable() {
    let cause = plain("cached plan must not change result type");
    let missing = UserReadFailure {
        user_id: "user-1".to_owned(),
        cause: Box::new(cause),
    };
    assert_eq!(
        user_read_error(&UserLookupError::UserMissing(missing)),
        SubjectReadError::Unavailable
    );
}

#[test]
fn user_missing_without_an_outage_is_other() {
    let missing = UserReadFailure {
        user_id: "user-1".to_owned(),
        cause: Box::new(plain("the user row did not match")),
    };
    assert_eq!(
        user_read_error(&UserLookupError::UserMissing(missing)),
        SubjectReadError::Other
    );
}

#[test]
fn user_prelookup_io_error_is_unavailable() {
    let error = UserLookupError::PreLookup(Box::new(IoError::other("the cache is unreachable")));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_prelookup_native_outage_is_unavailable() {
    let error = UserLookupError::PreLookup(Box::new(NativeDbError::<UserRowError>::NoDbConnected));
    assert_eq!(user_read_error(&error), SubjectReadError::Unavailable);
}

#[test]
fn user_prelookup_native_data_error_is_other() {
    let error = UserLookupError::PreLookup(Box::new(NativeDbError::<UserRowError>::CreateNull));
    assert_eq!(user_read_error(&error), SubjectReadError::Other);
}

#[test]
fn user_prelookup_data_fault_is_other() {
    let error = UserLookupError::PreLookup(Box::new(data_fault()));
    assert_eq!(user_read_error(&error), SubjectReadError::Other);
}

#[test]
fn user_prelookup_plain_error_is_other() {
    let error = UserLookupError::PreLookup(Box::new(plain("the backfill failed")));
    assert_eq!(user_read_error(&error), SubjectReadError::Other);
}

#[test]
fn identity_missing_key_is_not_found() {
    let error = IdentityStoreError::KeyLookup(KeyLookupError::MissingKey {
        key: "hashed-token".to_owned(),
    });
    assert_eq!(identity_error(&error), IdentityLookup::NotFound);
}

#[test]
fn identity_without_a_database_is_faulted() {
    let error = IdentityStoreError::KeyLookup(KeyLookupError::NoDatabase);
    assert_eq!(identity_error(&error), IdentityLookup::Faulted);
}

#[test]
fn identity_cache_error_is_faulted() {
    let error = IdentityStoreError::Cache(CredentialError::NonStringApiKey);
    assert_eq!(identity_error(&error), IdentityLookup::Faulted);
}

#[test]
fn identity_principal_without_a_source_key_is_faulted() {
    assert_eq!(
        identity_error(&IdentityStoreError::PrincipalMissingSourceKey),
        IdentityLookup::Faulted
    );
}
