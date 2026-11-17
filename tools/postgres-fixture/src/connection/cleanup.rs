use std::future::Future;

use tokio::task::JoinSet;
use tokio::time;

use super::{OPERATION_TIMEOUT, PostgresFixture};
use crate::error::FixtureError;

pub(super) async fn close_after(setup: FixtureError, fixture: PostgresFixture) -> FixtureError {
    match fixture.close_async().await {
        Ok(()) => setup,
        Err(cleanup_error) => FixtureError::SetupCleanup {
            setup: Box::new(setup),
            cleanup: Box::new(cleanup_error),
        },
    }
}

pub(super) async fn drain_drivers(
    drivers: &mut JoinSet<Result<(), FixtureError>>,
) -> Result<(), FixtureError> {
    let mut outcome = Ok(());
    while let Some(result) = drivers.join_next().await {
        outcome = combine_errors(
            outcome,
            result
                .map_err(FixtureError::DriverJoin)
                .and_then(|value| value),
        );
    }
    outcome
}

pub(super) async fn timed<F, T>(operation: &'static str, future: F) -> Result<T, FixtureError>
where
    F: Future<Output = T>,
{
    let result = time::timeout(OPERATION_TIMEOUT, future).await;
    result.map_err(|source| FixtureError::Timeout {
        operation,
        duration: OPERATION_TIMEOUT,
        source,
    })
}

pub(super) fn combine_errors(
    first_result: Result<(), FixtureError>,
    second_result: Result<(), FixtureError>,
) -> Result<(), FixtureError> {
    match (first_result, second_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first_error), Err(second_error)) => Err(FixtureError::MultipleCleanup {
            first: Box::new(first_error),
            second: Box::new(second_error),
        }),
    }
}
