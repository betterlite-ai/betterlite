use std::future::Future;
use std::pin::Pin;

use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::{ContainerAsync, GenericImage, ImageExt as _, runners::AsyncRunner as _};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

use super::{ConnectionOptions, FixtureSource, quote_identifier};
use crate::connection::cleanup::{cleanup_setup_driver, drain_drivers, timed};
use crate::connection::tls::{TlsMode, tls_connector};
use crate::error::FixtureError;

pub(super) async fn source_connection(
    source: FixtureSource,
) -> Result<(ConnectionOptions, Option<ContainerAsync<GenericImage>>), FixtureError> {
    match source {
        FixtureSource::Configured(options) => Ok((*options, None)),
        FixtureSource::Testcontainers => {
            let image = GenericImage::new("postgres", "17")
                .with_wait_for(WaitFor::message_on_stdout("database system is ready"))
                .with_wait_for(WaitFor::seconds(2))
                .with_exposed_port(5432.tcp())
                .with_env_var("POSTGRES_USER", "fixture")
                .with_env_var("POSTGRES_PASSWORD", "fixture")
                .with_env_var("POSTGRES_DB", "postgres");
            let container = image.start().await.map_err(FixtureError::Container)?;
            let host = container
                .get_host()
                .await
                .map_err(FixtureError::Container)?;
            let port = container
                .get_host_port_ipv4(5432.tcp())
                .await
                .map_err(FixtureError::Container)?;
            let options = ConnectionOptions::from_url(&format!(
                "postgresql://fixture:fixture@{host}:{port}/postgres"
            ))?;
            Ok((options, Some(container)))
        }
    }
}

pub(super) async fn connect(
    options: &ConnectionOptions,
    drivers: &mut JoinSet<Result<(), FixtureError>>,
) -> Result<Client, FixtureError> {
    match options.tls_mode {
        TlsMode::Disable | TlsMode::Prefer => {
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

pub(super) async fn create_database(
    options: &ConnectionOptions,
    database: &str,
) -> Result<(), FixtureError> {
    let mut drivers = JoinSet::new();
    let client = connect(options, &mut drivers).await?;
    let statement = format!("CREATE DATABASE {}", quote_identifier(database));
    let created = timed("create database", client.batch_execute(&statement))
        .await
        .and_then(|result| result.map_err(FixtureError::Database));
    drop(client);
    let driver_result = drain_drivers(&mut drivers).await;
    created?;
    if let Err(error) = driver_result {
        return Err(cleanup_setup_driver(error, Ok(()), options, database).await);
    }
    Ok(())
}

pub(super) async fn run_fixture_operation<F>(
    options: &ConnectionOptions,
    admin: &ConnectionOptions,
    database: &str,
    operation: F,
) -> Result<(), FixtureError>
where
    F: Send
        + for<'client> FnOnce(
            &'client Client,
        ) -> Pin<
            Box<dyn Future<Output = Result<(), FixtureError>> + Send + 'client>,
        >,
{
    let mut drivers = JoinSet::new();
    let connection_result = connect(options, &mut drivers).await;
    let client = match connection_result {
        Ok(client) => client,
        Err(error) => return Err(cleanup_setup_driver(error, Ok(()), admin, database).await),
    };
    let operation_result = operation(&client).await;
    drop(client);
    let driver_error = drain_drivers(&mut drivers).await;
    if let Err(error) = operation_result {
        return Err(cleanup_setup_driver(error, Ok(()), admin, database).await);
    }
    if let Err(error) = driver_error {
        return Err(cleanup_setup_driver(error, Ok(()), admin, database).await);
    }
    Ok(())
}
