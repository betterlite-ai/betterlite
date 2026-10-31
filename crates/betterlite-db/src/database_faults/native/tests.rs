#![cfg(test)]

use super::*;
use crate::postgres_store::diagnostics::PoolFailure;
use crate::postgres_store::errors::DbError as NativeDbError;
use deadpool_postgres::TimeoutType;
use std::convert::Infallible;

#[test]
fn pool_timeout_and_closed_are_transport() {
    let timeout = NativeDbError::<Infallible>::Pool {
        source: PoolFailure {
            driver: PoolError::Timeout(TimeoutType::Wait),
        },
    };
    let closed = NativeDbError::<Infallible>::Pool {
        source: PoolFailure {
            driver: PoolError::Closed,
        },
    };
    assert!(is_native_transport_error(&timeout));
    assert!(is_native_transport_error(&closed));
}

#[test]
fn pool_hook_failure_is_not_transport() {
    let no_runtime = NativeDbError::<Infallible>::Pool {
        source: PoolFailure {
            driver: PoolError::NoRuntimeSpecified,
        },
    };
    assert!(!is_native_transport_error(&no_runtime));
}

#[test]
fn no_db_connected_is_transport() {
    assert!(is_native_transport_error(
        &NativeDbError::<Infallible>::NoDbConnected
    ));
}

#[test]
fn create_null_is_not_transport_and_has_no_sqlstate() {
    let value = NativeDbError::<Infallible>::CreateNull;
    assert!(!is_native_transport_error(&value));
    assert_eq!(native_sqlstate(&value), None);
}

#[rstest::rstest]
#[case::deadlock("40P01", NativeFault::Deadlock)]
#[case::read_only("25006", NativeFault::ReadOnlyTransaction)]
#[case::connection_exception("08000", NativeFault::ConnectionUnavailable)]
#[case::connection_failure("08006", NativeFault::ConnectionUnavailable)]
#[case::cannot_connect_now("57P03", NativeFault::ConnectionUnavailable)]
#[case::admin_shutdown("57P01", NativeFault::ConnectionUnavailable)]
#[case::crash_shutdown("57P02", NativeFault::ConnectionUnavailable)]
#[case::unique_violation("23505", NativeFault::Unclassified)]
#[case::syntax_error("42601", NativeFault::Unclassified)]
fn sqlstate_classification_matrix(#[case] code: &str, #[case] expected: NativeFault) {
    assert_eq!(classify_sqlstate(&SqlState::from_code(code)), expected);
}
