#![cfg(test)]

use std::iter;

use rstest::rstest;

use super::chain::{AsyncpgFault, Chain, Failure, Origin, has_outage_in_chain};
use super::classification::{is_permanent_fault, is_service_unavailable};
use super::message::{TRANSIENT_MESSAGE, database_unavailable_message};
use super::policy::{
    AllowRequestsOnOutage, HandleOutcome, PolicyError, allows_requests_during_outage,
    handle_db_exception,
};
use super::projection::{SERVICE_UNAVAILABLE_STATUS, service_unavailable_error};
use crate::database_faults::{DbError, ErrorType, is_database_transport_error};

fn failure(origin: Origin) -> Failure {
    Failure {
        origin,
        message: String::new(),
        cause: None,
        context: None,
    }
}

fn database(error_type: ErrorType, message: &str) -> Failure {
    failure(Origin::Database(DbError {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }))
}

fn engine_internal() -> Failure {
    let mut value = database(ErrorType::Other, "engine crashed");
    if let Origin::Database(error) = &mut value.origin {
        error
            .traceback_modules
            .push("prisma.engine.errors".to_owned());
    }
    value
}

fn with_cause(mut value: Failure, cause: usize) -> Failure {
    value.cause = Some(cause);
    value
}

fn linear(mut failures: Vec<Failure>) -> Chain {
    let last = failures.len().saturating_sub(1);
    failures
        .iter_mut()
        .take(last)
        .enumerate()
        .for_each(|(index, failure)| failure.cause = index.checked_add(1));
    Chain { failures }
}

#[test]
fn cause_precedes_context() {
    let root = with_cause(
        with_context(database(ErrorType::Data, "not an outage"), 2),
        1,
    );
    let prefer_cause = Chain {
        failures: vec![
            root,
            database(ErrorType::Connect, "connection refused"),
            database(ErrorType::Data, "also not an outage"),
        ],
    };
    assert_eq!(prefer_cause.find_outage(), prefer_cause.failures.get(1));

    let shadowing_root = with_cause(
        with_context(database(ErrorType::Data, "not an outage"), 2),
        1,
    );
    let hidden_context = Chain {
        failures: vec![
            shadowing_root,
            database(ErrorType::Data, "not an outage"),
            database(ErrorType::Connect, "connection refused"),
        ],
    };
    assert_eq!(hidden_context.find_outage(), None);

    let context_only = Chain {
        failures: vec![
            with_context(database(ErrorType::Data, "not an outage"), 1),
            database(ErrorType::Connect, "connection refused"),
        ],
    };
    assert_eq!(context_only.find_outage(), context_only.failures.get(1));
}

fn with_context(mut value: Failure, context: usize) -> Failure {
    value.context = Some(context);
    value
}

#[test]
fn chain_stops_at_twenty() {
    let reachable = linear(
        iter::repeat_with(|| database(ErrorType::Data, "not an outage"))
            .take(20)
            .chain([database(ErrorType::Connect, "connection refused")])
            .collect(),
    );
    assert_eq!(reachable.links().len(), 20);
    assert_eq!(reachable.find_outage(), reachable.failures.get(19));

    let beyond = linear(
        iter::repeat_with(|| database(ErrorType::Data, "not an outage"))
            .take(21)
            .chain([database(ErrorType::Connect, "connection refused")])
            .collect(),
    );
    assert_eq!(beyond.find_outage(), None);
}

#[test]
fn cyclic_chain_bounded() {
    let first = with_cause(database(ErrorType::Data, "not an outage"), 1);
    let second = with_cause(database(ErrorType::Data, "not an outage"), 0);
    let chain = Chain {
        failures: vec![first, second],
    };
    assert_eq!(chain.links().len(), 20);
    assert_eq!(chain.find_outage(), None);
    assert!(!has_outage_in_chain(&chain));
}

#[test]
fn base_exception_skipped() {
    let mut base = failure(Origin::BaseException);
    base.message = "cached plan must not change result type".to_owned();
    let chain = Chain {
        failures: vec![
            with_cause(base, 1),
            database(ErrorType::Data, "not an outage"),
        ],
    };
    assert!(is_service_unavailable(&chain.failures[0]));
    assert!(chain.outages().is_empty());
    assert_eq!(chain.find_outage(), None);
    assert!(!has_outage_in_chain(&chain));
}

#[rstest]
#[case::infrastructure(database(ErrorType::Prisma, "permission denied"))]
#[case::transport(database(ErrorType::Connect, "connection refused"))]
#[case::engine_internal(engine_internal())]
fn infrastructure_transport_engine(#[case] value: Failure) {
    assert!(is_service_unavailable(&value));
    let chain = Chain {
        failures: vec![value],
    };
    assert!(has_outage_in_chain(&chain));
}

#[test]
fn stale_plan_casefold() {
    let stale = database(
        ErrorType::Data,
        "ERROR: CACHED PLAN MUST NOT CHANGE RESULT TYPE IN PREPARED STATEMENT",
    );
    assert!(is_service_unavailable(&stale));
    let only_stale = Chain {
        failures: vec![stale.clone()],
    };
    assert_eq!(database_unavailable_message(&only_stale), TRANSIENT_MESSAGE);
    if let Origin::Database(error) = &stale.origin {
        assert!(!is_database_transport_error(error));
    }

    let prisma = database(ErrorType::Prisma, "Cached Plan Must Not Change Result Type");
    assert!(is_service_unavailable(&prisma));
    if let Origin::Database(error) = &prisma.origin {
        assert!(!is_database_transport_error(error));
    }
}

#[test]
fn os_and_timeout() {
    let os_error = failure(Origin::Os);
    let timeout = failure(Origin::AsyncTimeout);
    assert!(is_service_unavailable(&os_error));
    assert!(is_service_unavailable(&timeout));
    let policy = AllowRequestsOnOutage::Boolean(true);
    assert_eq!(
        handle_db_exception(&os_error, &policy),
        Ok(HandleOutcome::Rethrown(os_error.clone()))
    );
    assert_eq!(
        handle_db_exception(&timeout, &policy),
        Ok(HandleOutcome::Rethrown(timeout.clone()))
    );

    let deadline = database(ErrorType::Deadline, "lookup did not answer");
    assert!(is_service_unavailable(&deadline));
    assert!(!is_permanent_fault(&deadline));
    assert_eq!(
        handle_db_exception(&deadline, &policy),
        Ok(HandleOutcome::Suppressed)
    );
}

#[rstest]
#[case::postgres_connection(AsyncpgFault::PostgresConnection)]
#[case::interface(AsyncpgFault::Interface)]
fn asyncpg_connection_interface(#[case] fault: AsyncpgFault) {
    assert!(is_service_unavailable(&failure(Origin::Asyncpg(fault))));
}

#[test]
fn asyncpg_unavailable_false() {
    let unavailable = failure(Origin::AsyncpgUnavailable(AsyncpgFault::PostgresConnection));
    assert!(!is_service_unavailable(&unavailable));
    let stale = Failure {
        origin: Origin::AsyncpgUnavailable(AsyncpgFault::PostgresConnection),
        message: "cached plan must not change result type".to_owned(),
        cause: None,
        context: None,
    };
    assert!(is_service_unavailable(&stale));
}

#[rstest]
#[case::data(ErrorType::Data)]
#[case::unique_violation(ErrorType::UniqueViolation)]
#[case::foreign_key_violation(ErrorType::ForeignKeyViolation)]
#[case::record_not_found(ErrorType::RecordNotFound)]
fn data_error_not_outage(#[case] error_type: ErrorType) {
    assert!(!is_service_unavailable(&database(
        error_type,
        "invalid byte sequence for encoding UTF8"
    )));
}

#[test]
fn permanent_beats_outer_transport() {
    let chain = linear(vec![
        database(ErrorType::Connect, "connection refused"),
        database(ErrorType::Prisma, "engine crashed"),
    ]);
    assert_eq!(chain.find_outage(), chain.failures.get(1));
    let message = database_unavailable_message(&chain);
    assert!(message.contains("Prisma"));
    assert!(!message.contains("Please retry"));
}

#[test]
fn first_transient_selected() {
    let after_data = linear(vec![
        database(ErrorType::Data, "not an outage"),
        database(ErrorType::Connect, "connection refused"),
    ]);
    assert_eq!(after_data.find_outage(), after_data.failures.get(1));

    let first_of_two = linear(vec![
        database(ErrorType::Connect, "connection refused"),
        database(ErrorType::Read, "read failed"),
    ]);
    assert_eq!(first_of_two.find_outage(), first_of_two.failures.first());
}

#[test]
fn no_outage_none() {
    let chain = linear(vec![database(ErrorType::Data, "invalid byte sequence")]);
    assert_eq!(chain.find_outage(), None);
    assert!(!has_outage_in_chain(&chain));
    assert_eq!(database_unavailable_message(&chain), TRANSIENT_MESSAGE);
}

#[test]
fn transient_retry_message() {
    let chain = linear(vec![database(ErrorType::Connect, "connection refused")]);
    let message = database_unavailable_message(&chain);
    assert!(message.contains("Please retry"));
    assert!(!message.contains("deployment needs attention"));
}

#[test]
fn permanent_deployment_message() {
    let chain = linear(vec![database(ErrorType::Prisma, "engine crashed")]);
    let message = database_unavailable_message(&chain);
    assert!(message.contains("Prisma"));
    assert!(message.contains("not a transient outage"));
    assert!(message.contains("deployment needs attention"));
    assert!(!message.contains("Please retry"));
}

#[test]
fn service_unavailable_envelope() {
    let chain = linear(vec![database(ErrorType::Connect, "connection refused")]);
    let projected = service_unavailable_error(&chain);
    assert_eq!(projected.status, SERVICE_UNAVAILABLE_STATUS);
    assert_eq!(projected.error_type, "no_db_connection");
    assert_eq!(projected.param, "None");
    assert_eq!(projected.code, "503");
    assert_eq!(projected.message, database_unavailable_message(&chain));
}

#[test]
fn enabled_direct_connection_suppressed() {
    let connect = database(ErrorType::Connect, "connection refused");
    assert_eq!(
        handle_db_exception(&connect, &AllowRequestsOnOutage::Boolean(true)),
        Ok(HandleOutcome::Suppressed)
    );
}

#[test]
fn enabled_wrapped_outage_rethrown() {
    let chain = linear(vec![
        database(ErrorType::Data, "repository failed"),
        database(ErrorType::Connect, "connection refused"),
    ]);
    let root = &chain.failures[0];
    assert!(has_outage_in_chain(&chain));
    assert_eq!(
        handle_db_exception(root, &AllowRequestsOnOutage::Boolean(true)),
        Ok(HandleOutcome::Rethrown(root.clone()))
    );
}

#[rstest]
#[case::boolean_false(AllowRequestsOnOutage::Boolean(false))]
#[case::text_false(AllowRequestsOnOutage::Text("false".to_owned()))]
#[case::unrecognized(AllowRequestsOnOutage::Text("maybe".to_owned()))]
fn outage_policy_off(#[case] policy: AllowRequestsOnOutage) {
    let connect = database(ErrorType::Connect, "connection refused");
    assert_eq!(
        handle_db_exception(&connect, &policy),
        Ok(HandleOutcome::Rethrown(connect.clone()))
    );
}

#[test]
fn from_error_preserves_the_cause_chain() {
    let caused = DbError {
        error_type: ErrorType::Prisma,
        message: "engine crashed".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    };
    let root = DbError {
        error_type: ErrorType::Connect,
        message: "connection refused".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: Some(Box::new(caused)),
    };
    let chain = Chain::from_error(&root);
    assert_eq!(chain.links().len(), 2);
    assert_eq!(chain.find_outage(), chain.failures.get(1));
    assert!(database_unavailable_message(&chain).contains("Prisma"));

    let stale = DbError {
        error_type: ErrorType::Data,
        message: "cached plan must not change result type".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    };
    let stale_chain = Chain::from_error(&stale);
    assert!(is_service_unavailable(&stale_chain.failures[0]));

    let transport = DbError {
        error_type: ErrorType::Prisma,
        message: "cannot reach database server".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    };
    let transport_chain = Chain::from_error(&transport);
    assert!(!is_permanent_fault(&transport_chain.failures[0]));
}

#[test]
fn unsupported_policy_value_is_an_error() {
    assert_eq!(
        allows_requests_during_outage(&AllowRequestsOnOutage::Unsupported),
        Err(PolicyError::UnsupportedValue)
    );
    let connect = database(ErrorType::Connect, "connection refused");
    assert_eq!(
        handle_db_exception(&connect, &AllowRequestsOnOutage::Unsupported),
        Err(PolicyError::UnsupportedValue)
    );
    let data = database(ErrorType::Data, "invalid byte sequence");
    assert_eq!(
        handle_db_exception(&data, &AllowRequestsOnOutage::Unsupported),
        Ok(HandleOutcome::Rethrown(data.clone()))
    );
}

#[test]
fn permanent_fault_requires_infrastructure() {
    assert!(is_permanent_fault(&database(
        ErrorType::Prisma,
        "engine crashed"
    )));
    assert!(!is_permanent_fault(&database(
        ErrorType::Prisma,
        "cannot reach database server"
    )));
    assert!(!is_permanent_fault(&database(
        ErrorType::Data,
        "invalid byte sequence"
    )));
    assert!(!is_permanent_fault(&failure(Origin::Os)));
}
