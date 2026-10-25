use std::fmt;
use std::str::FromStr as _;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use testcontainers::{ContainerAsync, GenericImage};
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tokio_postgres::Config;
use uuid::Uuid;

use crate::error::FixtureError;
use crate::seed::{DatabaseFixture, FixtureSeed};
use crate::seed_sql;

mod cleanup;
mod operations;
mod tls;
use cleanup::{cleanup_database, combine_errors, drain_drivers, timed};
use operations::{create_database, run_fixture_operation, source_connection};
use tls::{TlsMode, query_value, tls_mode};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

pub trait DatabaseNameGenerator: Send {
    fn next_name(&mut self) -> String;
}

#[derive(Debug, Default)]
pub struct RandomDatabaseName;

impl DatabaseNameGenerator for RandomDatabaseName {
    fn next_name(&mut self) -> String {
        format!("betterlite_fixture_{}", Uuid::new_v4().simple())
    }
}

#[derive(Clone)]
pub struct ConnectionOptions {
    config: Config,
    url: SecretString,
    root_certificate: Option<String>,
    tls_mode: TlsMode,
}

impl fmt::Debug for ConnectionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionOptions")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl ConnectionOptions {
    #[must_use]
    pub fn redacted_url(&self) -> String {
        let redacted = format!("{:?}", self.config);
        redacted
    }

    #[must_use]
    pub fn database_url(&self) -> SecretString {
        if self.url.expose_secret().is_empty() {
            return SecretString::from(String::new());
        }
        self.url.clone()
    }

    #[must_use]
    pub const fn uses_tls(&self) -> bool {
        match self.tls_mode {
            TlsMode::Disable | TlsMode::Prefer => false,
            TlsMode::Require | TlsMode::VerifyCa | TlsMode::VerifyFull => true,
        }
    }

    fn from_url(raw: &str) -> Result<Self, FixtureError> {
        let tls_mode = tls_mode(raw);
        let config_source = tls::normalized_config_source(raw, tls_mode);
        let config = Config::from_str(&config_source)
            .map_err(|source| FixtureError::InvalidConfiguredUrl { source })?;
        Ok(Self {
            config,
            url: SecretString::from(raw.to_owned()),
            root_certificate: query_value(raw, "sslrootcert"),
            tls_mode,
        })
    }

    fn with_database(&self, database: &str) -> Self {
        let mut config = self.config.clone();
        config.dbname(database);
        let raw = self.url.expose_secret();
        let url = raw.split_once("://").map_or_else(
            || raw.to_owned(),
            |(scheme, remainder)| {
                let (authority, suffix) = remainder.split_once('/').unwrap_or((remainder, ""));
                let query = suffix.split_once('?').map_or("", |(_, query)| query);
                format!("{scheme}://{authority}/{database}{query}")
            },
        );
        Self {
            config,
            url: SecretString::from(url),
            root_certificate: self.root_certificate.clone(),
            tls_mode: self.tls_mode,
        }
    }
}

#[derive(Debug, Clone)]
pub enum FixtureSource {
    Configured(Box<ConnectionOptions>),
    Testcontainers,
}

pub fn select_source(configured_url: Option<&str>) -> Result<FixtureSource, FixtureError> {
    match configured_url {
        Some("") => Err(FixtureError::EmptyConfiguredUrl),
        Some(url) => Ok(FixtureSource::Configured(Box::new(
            ConnectionOptions::from_url(url)?,
        ))),
        None => Ok(FixtureSource::Testcontainers),
    }
}

pub struct PostgresFixture {
    pub options: ConnectionOptions,
    pub database: String,
    pub container: Option<ContainerAsync<GenericImage>>,
    pub drivers: JoinSet<Result<(), FixtureError>>,
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
        let mut generator = RandomDatabaseName;
        runtime.block_on(Self::provision_with_name_generator(
            configured_url,
            &fixture.seed,
            &mut generator,
        ))
    }

    pub async fn provision_async(
        configured_url: Option<&str>,
        seed: &FixtureSeed,
    ) -> Result<Self, FixtureError> {
        let mut generator = RandomDatabaseName;
        Self::provision_with_name_generator(configured_url, seed, &mut generator).await
    }

    pub async fn provision_with_name_generator<G>(
        configured_url: Option<&str>,
        seed: &FixtureSeed,
        generator: &mut G,
    ) -> Result<Self, FixtureError>
    where
        G: DatabaseNameGenerator,
    {
        seed.validate()?;
        let source = select_source(configured_url)?;
        let (admin, container) = source_connection(source).await?;
        let database = generator.next_name();
        let admin_url = admin.with_database("postgres");
        let create_database_name = database.clone();
        run_fixture_operation(&admin_url, &admin, &database, move |client| {
            Box::pin(async move {
                match timed(
                    "create database",
                    create_database(client, &create_database_name),
                )
                .await?
                {
                    Ok(()) => Ok(()),
                    Err(error) => Err(error),
                }
            })
        })
        .await?;
        let options = admin.with_database(&database);
        let seed_copy = seed.clone();
        run_fixture_operation(&options, &admin, &database, move |client| {
            Box::pin(async move {
                match timed("seed database", seed_sql::seed_database(client, &seed_copy)).await? {
                    Ok(()) => Ok(()),
                    Err(error) => Err(error),
                }
            })
        })
        .await?;
        Ok(Self {
            options,
            database,
            container,
            drivers: JoinSet::new(),
        })
    }

    pub fn close(self) -> Result<(), FixtureError> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(FixtureError::Runtime)?;
        runtime.block_on(self.close_async())
    }

    async fn close_async(mut self) -> Result<(), FixtureError> {
        let driver_error = drain_drivers(&mut self.drivers).await;
        let admin = self.options.with_database("postgres");
        let cleanup = cleanup_database(&admin, &self.database).await;
        drop(self.container);
        combine_errors(driver_error, cleanup)
    }
}
pub(crate) fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}
