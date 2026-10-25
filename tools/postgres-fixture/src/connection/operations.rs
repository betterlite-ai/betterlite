use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::{ContainerAsync, GenericImage, ImageExt as _, runners::AsyncRunner as _};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

use super::{ConnectionOptions, FixtureSource, resource::Resource};
use crate::connection::cleanup::{cleanup_setup_driver, combine_errors, drain_drivers, timed};
use crate::connection::tls::{TlsMode, tls_connector};
use crate::error::FixtureError;
use crate::{seed::FixtureSeed, seed_sql};

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

pub(super) async fn create_resource(
    options: &ConnectionOptions,
    resource: &Resource,
) -> Result<(), FixtureError> {
    let mut drivers = JoinSet::new();
    let client = connect(options, &mut drivers).await?;
    let statement = resource.create_statement();
    let created = timed("create resource", client.batch_execute(&statement))
        .await
        .and_then(|result| result.map_err(FixtureError::Database));
    drop(client);
    let driver_result = drain_drivers(&mut drivers).await;
    created?;
    if let Err(error) = driver_result {
        return Err(cleanup_setup_driver(error, options, resource).await);
    }
    Ok(())
}

pub(super) async fn seed_fixture(
    options: &ConnectionOptions,
    admin: &ConnectionOptions,
    resource: &Resource,
    seed: &FixtureSeed,
) -> Result<(), FixtureError> {
    let mut drivers = JoinSet::new();
    let connection_result = connect(options, &mut drivers).await;
    let client = match connection_result {
        Ok(client) => client,
        Err(error) => return Err(cleanup_setup_driver(error, admin, resource).await),
    };
    let operation_result = timed("seed database", seed_sql::seed_database(&client, seed))
        .await
        .and_then(|result| result);
    drop(client);
    let driver_error = drain_drivers(&mut drivers).await;
    if let Err(error) = combine_errors(operation_result, driver_error) {
        return Err(cleanup_setup_driver(error, admin, resource).await);
    }
    Ok(())
}
