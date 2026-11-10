use std::fs;
use std::io::Read as _;
use std::path::Path;
use std::time::Instant;

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::fixtures::http_scenarios::{
    ScenarioSetup, SetupJournalEntry, apply_setup,
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;

use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::run::Clock;
use crate::http_scenarios::run::{FixtureRepository, PhysicalNames};

const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn read_log(path: &Path) -> Result<String, ScenarioError> {
    let file = fs::File::open(path).map_err(|source| ScenarioError::Artifact {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_LOG_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|source| ScenarioError::Artifact {
            path: path.to_owned(),
            source,
        })?;
    String::from_utf8(bytes).map_err(|source| ScenarioError::LogUtf8 {
        path: path.to_owned(),
        source,
    })
}

pub(super) struct PostgresRepository {
    pub(super) configured_url: Option<SecretString>,
    pub(super) fixture: Option<PostgresFixture>,
    pub(super) prefix: String,
    pub(super) redis_url: String,
}

impl PostgresRepository {
    fn provision_database(&mut self) -> Result<String, ScenarioError> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(ScenarioError::Io)?;
        let entered = runtime.enter();
        let mut task = PostgresFixture::provision_schema(self.configured_url.as_ref());
        let fixture = runtime
            .block_on(task.resolve())
            .map_err(ScenarioError::Fixture)?;
        drop(entered);
        let url = fixture.options.url.expose_secret().to_owned();
        self.fixture = Some(fixture);
        Ok(url)
    }
}

impl FixtureRepository for PostgresRepository {
    fn provision(&mut self, need_database: bool) -> Result<PhysicalNames, ScenarioError> {
        let database_url = if need_database {
            self.provision_database()?
        } else {
            String::new()
        };
        Ok(PhysicalNames {
            database_url: SecretString::from(database_url),
            redis_url: self.redis_url.clone(),
            redis_prefix: self.prefix.clone(),
        })
    }

    fn insert(&mut self, setup: &ScenarioSetup) -> Result<Vec<SetupJournalEntry>, ScenarioError> {
        self.fixture.as_ref().map_or_else(
            || Ok(Vec::new()),
            |fixture| apply_setup(&fixture.options, setup).map_err(ScenarioError::Fixture),
        )
    }

    fn close(&mut self) -> Result<(), ScenarioError> {
        self.fixture.take().map_or_else(
            || Ok(()),
            |fixture| fixture.close().map_err(ScenarioError::Fixture),
        )
    }
}

pub(super) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
