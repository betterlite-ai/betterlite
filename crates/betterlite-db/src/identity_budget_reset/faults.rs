use std::convert::Infallible;
use std::error::Error;

use crate::database_faults::native::is_native_transport_error;
use crate::database_faults::{DbError, ErrorType};
use crate::postgres_store::errors::DbError as NativeError;
use crate::spend_reset_unit_of_work::UnitOfWorkError;
use crate::spend_reset_unit_of_work::batch::CommitError;

pub(crate) type Fault = DbError;

pub(crate) fn describe<E>(error: &E) -> String
where
    E: Error,
{
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        cause = inner.source();
    }
    message
}

// A native transport failure is a connect fault for the read retry, which replays every transport error.
pub(crate) fn read_fault(error: &NativeError<Infallible>) -> Fault {
    let kind = if is_native_transport_error(error) {
        ErrorType::Connect
    } else {
        ErrorType::Other
    };
    fault(kind, describe(error))
}

pub(crate) fn write_fault(error: UnitOfWorkError<Infallible>) -> Fault {
    match error {
        UnitOfWorkError::Body(never) => match never {},
        UnitOfWorkError::Commit(commit) => commit_fault(&commit),
    }
}

fn commit_fault(error: &CommitError) -> Fault {
    match error {
        CommitError::Statement(source) => statement_fault(source),
        CommitError::Rejected(source) => fault(ErrorType::Other, describe(source)),
        CommitError::OutcomeUnknown(source) => fault(ErrorType::ReadTimeout, describe(source)),
        CommitError::MissingResetTarget { table } => fault(
            ErrorType::Other,
            format!("no row matched the reset target in {table}"),
        ),
    }
}

// DB_RETRY_SAFE_ERROR_TYPES holds only ConnectError: a pool that handed out no connection proves the write never reached Postgres.
fn statement_fault(source: &NativeError<Infallible>) -> Fault {
    let safe = matches!(
        source,
        NativeError::NoDbConnected | NativeError::Pool { .. }
    );
    let kind = if safe {
        ErrorType::Connect
    } else {
        ErrorType::Other
    };
    fault(kind, describe(source))
}

const fn fault(kind: ErrorType, message: String) -> Fault {
    Fault {
        error_type: kind,
        message,
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
