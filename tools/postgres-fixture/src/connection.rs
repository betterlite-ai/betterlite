use std::fmt;
use std::str::FromStr as _;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use testcontainers::{ContainerAsync, GenericImage};
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tokio_postgres::{Config, config::SslMode};
use url::Url;
use uuid::Uuid;

use crate::error::FixtureError;
use crate::seed::{DatabaseFixture, FixtureSeed};

mod cleanup;
mod operations;
pub mod resource;
use resource::{Ownership, Resource};
pub mod provision;
use provision::ProvisionTask;
mod tls;
use cleanup::{cleanup_resource, combine_errors};

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
    parsed_url: Url,
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
    pub fn database_url(&self) -> SecretString {
        let secret_url = self.url.expose_secret();
        SecretString::from(secret_url.to_owned())
    }

    fn from_url(raw: &str) -> Result<Self, FixtureError> {
        let tls_mode = tls_mode(raw)?;
        let config_source = tls::normalized_config_source(raw, tls_mode);
        let mut config = Config::from_str(&config_source)
            .map_err(|source| FixtureError::InvalidConfiguredUrl { source })?;
        config.ssl_mode(match tls_mode {
            TlsMode::Disable => SslMode::Disable,
            TlsMode::Prefer => SslMode::Prefer,
            TlsMode::Require | TlsMode::VerifyCa | TlsMode::VerifyFull => SslMode::Require,
        });
        let parsed_url = Url::parse(raw).map_err(|source| FixtureError::InvalidUrl { source })?;
        Ok(Self {
            config,
            url: SecretString::from(raw.to_owned()),
            parsed_url,
            root_certificate: query_value(raw, "sslrootcert"),
            tls_mode,
        })
    }

    fn with_database(&self, database: &str) -> Self {
        let mut options = self.clone();
        options.config.dbname(database);
        options.parsed_url.set_path(&format!("/{database}"));
        options.url = SecretString::from(options.parsed_url.to_string());
        options
    }
}

#[derive(Debug, Clone)]
pub enum FixtureSource {
    Configured(Box<ConnectionOptions>),
    Testcontainers,
}

pub fn select_source(configured_url: Option<&SecretString>) -> Result<FixtureSource, FixtureError> {
    match configured_url {
        Some(url) if url.expose_secret().is_empty() => Err(FixtureError::EmptyConfiguredUrl),
        Some(url) => Ok(FixtureSource::Configured(Box::new(
            ConnectionOptions::from_url(url.expose_secret())?,
        ))),
        None => Ok(FixtureSource::Testcontainers),
    }
}

pub struct PostgresFixture {
    pub options: ConnectionOptions,
    pub ownership: Ownership,
    pub container: Option<ContainerAsync<GenericImage>>,
}

impl fmt::Debug for PostgresFixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresFixture")
            .field("options", &self.options)
            .field("ownership", &self.ownership)
            .finish_non_exhaustive()
    }
}

impl PostgresFixture {
    pub fn provision_from_url(
        configured_url: Option<&SecretString>,
        fixture: &DatabaseFixture,
    ) -> Result<Self, FixtureError> {
        fixture.seed.validate()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(FixtureError::Runtime)?;
        let mut generator = RandomDatabaseName;
        let _runtime_guard = runtime.enter();
        let mut task =
            Self::provision_with_name_generator(configured_url, &fixture.seed, &mut generator);
        runtime.block_on(task.resolve())
    }

    pub fn provision_with_name_generator<G>(
        configured_url: Option<&SecretString>,
        seed: &FixtureSeed,
        generator: &mut G,
    ) -> ProvisionTask
    where
        G: DatabaseNameGenerator,
    {
        let mut tasks = JoinSet::new();
        tasks.spawn(provision::provision_inner(
            configured_url.cloned(),
            seed.clone(),
            Resource::Database(generator.next_name()),
        ));
        ProvisionTask { tasks }
    }

    pub fn provision_schema(configured_url: Option<&SecretString>) -> ProvisionTask {
        let mut tasks = JoinSet::new();
        tasks.spawn(provision::provision_inner(
            configured_url.cloned(),
            FixtureSeed::default(),
            Resource::Schema(RandomDatabaseName.next_name()),
        ));
        ProvisionTask { tasks }
    }

    pub fn close(self) -> Result<(), FixtureError> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(FixtureError::Runtime)?;
        runtime.block_on(self.close_async())
    }

    pub async fn close_async(self) -> Result<(), FixtureError> {
        let cleanup = cleanup_resource(&self.ownership.admin, &self.ownership.resource).await;
        let container_cleanup = match self.container {
            Some(container) => container.rm().await.map_err(FixtureError::Container),
            None => Ok(()),
        };
        combine_errors(cleanup, container_cleanup)
    }
}

pub(crate) fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}
