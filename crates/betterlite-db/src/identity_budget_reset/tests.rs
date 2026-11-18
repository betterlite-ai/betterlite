#![cfg(test)]

pub(crate) mod support;

use std::convert::Infallible;

use crate::database_faults::ErrorType;
use crate::postgres_store::errors::DbError as NativeError;
use crate::spend_reset_unit_of_work::UnitOfWorkError;
use crate::spend_reset_unit_of_work::batch::CommitError;

use super::faults::{describe, read_fault, write_fault};
use super::phase::IdentityPhase;

#[test]
fn phase_accessors_match_the_litellm_identifiers() {
    let keys = IdentityPhase::Keys;
    assert_eq!(keys.call_type(), "reset_budget_keys");
    assert_eq!(keys.subject(), "keys");
    assert_eq!(keys.row_subject(), "key");
    assert_eq!(keys.found_count_key(), "num_keys_found");
    assert_eq!(keys.updated_count_key(), "num_keys_updated");
    assert_eq!(keys.failed_count_key(), "num_keys_failed");
    assert_eq!(keys.read_reason(), "reset_budget_read_keys_failure");
    assert_eq!(keys.write_reason(), "reset_budget_write_keys_failure");
    let users = IdentityPhase::Users;
    assert_eq!(users.call_type(), "reset_budget_users");
    assert_eq!(users.subject(), "users");
    assert_eq!(users.row_subject(), "user");
    assert_eq!(users.found_count_key(), "num_users_found");
    assert_eq!(users.updated_count_key(), "num_users_updated");
    assert_eq!(users.failed_count_key(), "num_users_failed");
    assert_eq!(users.read_reason(), "reset_budget_read_users_failure");
    assert_eq!(users.write_reason(), "reset_budget_write_users_failure");
}

#[derive(Debug, thiserror::Error)]
#[error("the inner failure")]
struct InnerFailure;

#[derive(Debug, thiserror::Error)]
#[error("the middle failure")]
struct MiddleFailure(#[source] InnerFailure);

#[derive(Debug, thiserror::Error)]
#[error("the outer failure")]
struct OuterFailure(#[source] MiddleFailure);

#[test]
fn faults_describe_joins_the_whole_cause_chain() {
    let described = describe(&OuterFailure(MiddleFailure(InnerFailure)));
    assert_eq!(
        described,
        "the outer failure: the middle failure: the inner failure"
    );
}

#[test]
fn faults_read_keeps_a_missing_connection_retryable() {
    let fault = read_fault(&NativeError::<Infallible>::NoDbConnected);
    assert_eq!(fault.error_type, ErrorType::Connect);
}

#[test]
fn faults_read_marks_a_statement_error_as_other() {
    let fault = read_fault(&NativeError::<Infallible>::CreateNull);
    assert_eq!(fault.error_type, ErrorType::Other);
}

#[test]
fn faults_write_retries_only_a_missing_connection() {
    let missing =
        UnitOfWorkError::<Infallible>::Commit(CommitError::Statement(NativeError::NoDbConnected));
    assert_eq!(write_fault(&missing).error_type, ErrorType::Connect);
    let statement =
        UnitOfWorkError::<Infallible>::Commit(CommitError::Statement(NativeError::CreateNull));
    assert_eq!(write_fault(&statement).error_type, ErrorType::Other);
}

#[test]
fn faults_write_marks_a_rejected_commit_as_other() {
    let rejected =
        UnitOfWorkError::<Infallible>::Commit(CommitError::Rejected(NativeError::CreateNull));
    assert_eq!(write_fault(&rejected).error_type, ErrorType::Other);
}

#[test]
fn faults_write_marks_an_unknown_outcome_as_a_read_timeout() {
    let unknown =
        UnitOfWorkError::<Infallible>::Commit(CommitError::OutcomeUnknown(NativeError::CreateNull));
    assert_eq!(write_fault(&unknown).error_type, ErrorType::ReadTimeout);
}

#[test]
fn faults_write_names_the_missing_reset_target() {
    let missing = UnitOfWorkError::<Infallible>::Commit(CommitError::MissingResetTarget {
        table: "LiteLLM_UserTable",
    });
    let fault = write_fault(&missing);
    assert_eq!(fault.error_type, ErrorType::Other);
    assert_eq!(
        fault.message,
        "no row matched the reset target in LiteLLM_UserTable"
    );
}
