use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use secrecy::{ExposeSecret as _, SecretString};
use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::{ContainerAsync, GenericImage, ImageExt as _, runners::AsyncRunner as _};
use tokio::runtime::{Builder, Handle};
use tokio_postgres::{Client, NoTls};
use url::Url;

use crate::error::FixtureError;
use crate::seed::{DatabaseFixture, FixtureSeed};
use crate::seed_sql;

static DATABASE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct ConnectionOptions {
    url: SecretString,
}

impl fmt::Debug for ConnectionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionOptions")
            .field("url", &self.redacted_url())
            .finish()
    }
}

impl ConnectionOptions {
    #[must_use]
    pub fn redacted_url(&self) -> String {
        let Ok(mut parsed) = Url::parse(self.url.expose_secret()) else {
            return "postgresql://[invalid]".to_owned();
        };
        if parsed.set_password(None).is_err() {
            return "postgresql://[invalid]".to_owned();
        }
        if !parsed.username().is_empty() && parsed.set_username("[redacted]").is_err() {
            return "postgresql://[invalid]".to_owned();
        }
        parsed.to_string()
    }

    fn from_url(raw: &str) -> Result<Self, FixtureError> {
        let parsed = Url::parse(raw).map_err(|_error| FixtureError::InvalidConfiguredUrl)?;
        if !matches!(parsed.scheme(), "postgres" | "postgresql") || parsed.host_str().is_none() {
            return Err(FixtureError::InvalidConfiguredUrl);
        }
        Ok(Self {
            url: SecretString::from(raw.to_owned()),
        })
    }

    fn with_database(&self, database: &str) -> Result<Self, FixtureError> {
        let mut parsed = Url::parse(self.url.expose_secret())
            .map_err(|_error| FixtureError::InvalidConfiguredUrl)?;
        parsed.set_path(&format!("/{database}"));
        Self::from_url(parsed.as_str())
    }
}

#[derive(Debug, Clone)]
pub enum FixtureSource {
    Configured(ConnectionOptions),
    Testcontainers,
}

pub fn select_source(configured_url: Option<&str>) -> Result<FixtureSource, FixtureError> {
    match configured_url {
        Some("") => Err(FixtureError::EmptyConfiguredUrl),
        Some(url) => Ok(FixtureSource::Configured(ConnectionOptions::from_url(url)?)),
        None => Ok(FixtureSource::Testcontainers),
    }
}

pub struct PostgresFixture {
    pub options: ConnectionOptions,
    pub database: String,
    pub container: Option<ContainerAsync<GenericImage>>,
}

impl fmt::Debug for PostgresFixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresFixture")
            .field("options", &self.options)
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

impl PostgresFixture {
    pub fn provision_from_url(
        configured_url: Option<&str>,
        fixture: &DatabaseFixture,
    ) -> Result<Self, FixtureError> {
        fixture.seed.validate()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(FixtureError::Runtime)?;
        runtime.block_on(Self::provision_async(configured_url, &fixture.seed))
    }

    pub async fn provision_async(
        configured_url: Option<&str>,
        seed: &FixtureSeed,
    ) -> Result<Self, FixtureError> {
        seed.validate()?;
        let source = select_source(configured_url)?;
        let (admin, container) = source_connection(source).await?;
        let database = new_database_name()?;
        let admin_url = admin.with_database("postgres")?;
        let admin_client = connect(&admin_url).await?;
        create_database(&admin_client, &database).await?;
        drop(admin_client);
        let options = admin.with_database(&database)?;
        let client = connect(&options).await?;
        seed_sql::seed_database(&client, seed).await?;
        drop(client);
        Ok(Self {
            options,
            database,
            container,
        })
    }

    pub fn close(self) -> Result<(), FixtureError> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(FixtureError::Runtime)?;
        runtime.block_on(self.drop_database())
    }

    async fn drop_database(self) -> Result<(), FixtureError> {
        let admin = self.options.with_database("postgres")?;
        let client = connect(&admin).await?;
        let statement = format!(
            "DROP DATABASE IF EXISTS {}",
            quote_identifier(&self.database)
        );
        client
            .batch_execute(&statement)
            .await
            .map_err(FixtureError::Database)?;
        drop(client);
        drop(self.container);
        Ok(())
    }
}

async fn connect(options: &ConnectionOptions) -> Result<Client, FixtureError> {
    let (client, connection) = tokio_postgres::connect(options.url.expose_secret(), NoTls)
        .await
        .map_err(FixtureError::Connect)?;
    Handle::current().spawn(async move {
        drop(connection.await);
    });
    Ok(client)
}

async fn source_connection(
    source: FixtureSource,
) -> Result<(ConnectionOptions, Option<ContainerAsync<GenericImage>>), FixtureError> {
    match source {
        FixtureSource::Configured(options) => Ok((options, None)),
        FixtureSource::Testcontainers => {
            let image = GenericImage::new("postgres", "16.6")
                .with_wait_for(WaitFor::message_on_stdout("database system is ready"))
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

async fn create_database(client: &Client, database: &str) -> Result<(), FixtureError> {
    let statement = format!("CREATE DATABASE {}", quote_identifier(database));
    client
        .batch_execute(&statement)
        .await
        .map_err(FixtureError::Database)
}

fn new_database_name() -> Result<String, FixtureError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_error| FixtureError::DatabaseName)?
        .as_nanos();
    let count = DATABASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(format!("betterlite_fixture_{nanos:x}_{count:x}"))
}

pub(crate) fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}
