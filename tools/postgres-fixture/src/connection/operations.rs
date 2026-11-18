use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

use super::resource::{Ownership, Resource};
use super::{ConnectionOptions, PostgresFixture};
use crate::connection::cleanup::{close_after, combine_errors, drain_drivers, timed};
use crate::connection::tls::{TlsMode, tls_connector};
use crate::error::FixtureError;
use crate::{seed::FixtureSeed, seed_sql};

pub(super) enum Work<'input> {
    Statements(&'input [String]),
    Seed(&'input FixtureSeed),
}

pub(super) async fn connect(
    options: &ConnectionOptions,
    drivers: &mut JoinSet<Result<(), FixtureError>>,
) -> Result<Client, FixtureError> {
    match options.tls_mode {
        TlsMode::Disable => {
            let connected = timed("connect", options.config.connect(NoTls))
                .await?
                .map_err(FixtureError::Connect)?;
            let (client, connection) = connected;
            drivers.spawn(async move { connection.await.map_err(FixtureError::Driver) });
            Ok(client)
        }
        TlsMode::Require | TlsMode::VerifyCa | TlsMode::VerifyFull => {
            let connector = tls_connector(options)?;
            let connected = timed("connect", options.config.connect(connector))
                .await?
                .map_err(FixtureError::Connect)?;
            let (client, connection) = connected;
            drivers.spawn(async move { connection.await.map_err(FixtureError::Driver) });
            Ok(client)
        }
    }
}

pub(super) async fn create_resource(
    admin: &ConnectionOptions,
    resource: Resource,
    statement: &str,
) -> Result<PostgresFixture, FixtureError> {
    let options = admin.with_database(&resource.0)?;
    let mut drivers = JoinSet::new();
    let client = connect(admin, &mut drivers).await?;
    let created = timed("create resource", client.batch_execute(statement))
        .await
        .and_then(|result| result.map_err(FixtureError::Database));
    drop(client);
    let driver_result = drain_drivers(&mut drivers).await;
    created?;
    let fixture = PostgresFixture {
        options,
        ownership: Ownership {
            resource,
            admin: Box::new(admin.clone()),
        },
    };
    if let Err(error) = driver_result {
        return Err(close_after(error, fixture).await);
    }
    Ok(fixture)
}

pub(super) async fn fill_resource(
    fixture: PostgresFixture,
    work: Work<'_>,
) -> Result<PostgresFixture, FixtureError> {
    let mut drivers = JoinSet::new();
    let connection_result = connect(&fixture.options, &mut drivers).await;
    let client = match connection_result {
        Ok(client) => client,
        Err(error) => return Err(close_after(error, fixture).await),
    };
    let operation_result = timed("fill database", run(&client, work))
        .await
        .and_then(|result| result);
    drop(client);
    let driver_error = drain_drivers(&mut drivers).await;
    if let Err(error) = combine_errors(operation_result, driver_error) {
        return Err(close_after(error, fixture).await);
    }
    Ok(fixture)
}

async fn run(client: &Client, work: Work<'_>) -> Result<(), FixtureError> {
    match work {
        Work::Statements(statements) => {
            for statement in statements {
                client
                    .batch_execute(statement)
                    .await
                    .map_err(FixtureError::Database)?;
            }
            Ok(())
        }
        Work::Seed(seed) => seed_sql::seed_database(client, seed).await,
    }
}
