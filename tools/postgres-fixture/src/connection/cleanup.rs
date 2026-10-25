use std::future::Future;

use tokio::task::JoinSet;
use tokio::time;

use super::{ConnectionOptions, OPERATION_TIMEOUT, connect, quote_identifier};
use crate::error::FixtureError;

pub(super) async fn cleanup_database(
    options: &ConnectionOptions,
    database: &str,
) -> Result<(), FixtureError> {
    let mut drivers = JoinSet::new();
    let client = connect(options, &mut drivers).await?;
    let statement = format!("DROP DATABASE IF EXISTS {}", quote_identifier(database));
    let query = timed("drop database", client.batch_execute(&statement))
        .await
        .and_then(|result| result.map_err(FixtureError::Database));
    drop(client);
    combine_errors(query, drain_drivers(&mut drivers).await)
}

pub(super) async fn cleanup_setup_driver(
    setup: FixtureError,
    driver: Result<(), FixtureError>,
    admin: &ConnectionOptions,
    database: &str,
) -> FixtureError {
    let cleanup_result = combine_errors(driver, cleanup_database(admin, database).await);
    match cleanup_result {
        Ok(()) => setup,
        Err(cleanup_error) => FixtureError::SetupCleanup {
            setup: Box::new(setup),
            cleanup: Box::new(cleanup_error),
        },
    }
}

pub(super) async fn drain_drivers(
    drivers: &mut JoinSet<Result<(), tokio_postgres::Error>>,
) -> Result<(), FixtureError> {
    let mut first = None;
    while let Some(result) = drivers.join_next().await {
        let error = match result {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => FixtureError::Driver(error),
            Err(error) => FixtureError::DriverJoin(error),
        };
        if first.is_none() {
            first = Some(error);
        }
    }
    first.map_or(Ok(()), Err)
}

pub(super) async fn timed<T>(
    operation: &'static str,
    future: impl Future<Output = T>,
) -> Result<T, FixtureError> {
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
