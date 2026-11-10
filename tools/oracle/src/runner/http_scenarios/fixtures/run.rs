use std::env::temp_dir;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use secrecy::SecretString;

use self::redis::RedisServer;
use super::adapters::PostgresRepository;
use super::adapters::SystemClock;
use super::gateway::GatewayPeerFactory;
use crate::OracleError;
use crate::case::UpstreamAnswer;
use crate::cli::Options;
use crate::http;
use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::model::HttpScenario;
use crate::http_scenarios::run::{PeerFixture, ScenarioDependencies, run};
use crate::mock::MockProvider;
use crate::paths::unique_id;
use crate::routing_mock_scripts::MockScript;
use crate::runner::DatabaseEnvironment;

#[path = "run/polling.rs"]
mod polling;
#[path = "run/redis.rs"]
mod redis;
#[path = "run/tests.rs"]
mod tests;

struct ScenarioEnvironment {
    provider: Arc<MockProvider>,
    group_dir: PathBuf,
    configured_url: Option<SecretString>,
    redis: RedisServer,
    _scratch: tempfile::TempDir,
}

impl ScenarioEnvironment {
    fn start(scenario: &HttpScenario) -> Result<Self, OracleError> {
        let environment =
            <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])?;
        let answers = scenario
            .steps
            .iter()
            .flat_map(|step| step.upstream.iter().cloned())
            .collect::<Vec<UpstreamAnswer>>();
        let script = MockScript::try_from(answers.as_slice())?;
        let provider = Arc::new(MockProvider::start(script)?);
        let redis = RedisServer::start()?;
        let scratch = tempfile::Builder::new()
            .prefix("oracle-scenario-")
            .tempdir()
            .map_err(|source| OracleError::Filesystem {
                path: temp_dir(),
                source,
            })?;
        let group_dir = scratch.path().to_path_buf();
        Ok(Self {
            provider,
            group_dir,
            configured_url: environment.postgres_test_url,
            redis,
            _scratch: scratch,
        })
    }
}

pub(crate) fn run_case(
    options: &Options,
    root: &Path,
    scenario: &HttpScenario,
) -> Result<bool, OracleError> {
    let mut environment = ScenarioEnvironment::start(scenario)?;
    let mut factory = GatewayPeerFactory {
        options,
        provider: Arc::clone(&environment.provider),
        client: http::client()?,
        group_dir: environment.group_dir.clone(),
    };
    let (mut first_repository, mut second_repository) = repositories(&mut environment)?;
    let mut first_polling = environment.redis.polling();
    let mut second_polling = environment.redis.polling();
    let outcome = run(ScenarioDependencies {
        scenario,
        clock: &SystemClock,
        factory: &mut factory,
        first: PeerFixture {
            repository: &mut first_repository,
            polling: &mut first_polling,
        },
        second: PeerFixture {
            repository: &mut second_repository,
            polling: &mut second_polling,
        },
        upstream: &environment.provider.base_url,
        artifact_root: root,
    })?;
    ensure_supported(&environment.redis)?;
    Ok(outcome)
}

fn ensure_supported(redis: &RedisServer) -> Result<(), ScenarioError> {
    redis.unsupported().map_or_else(
        || Ok(()),
        |command| Err(ScenarioError::UnknownCommand { command }),
    )
}

fn repositories(
    environment: &mut ScenarioEnvironment,
) -> Result<(PostgresRepository, PostgresRepository), OracleError> {
    let redis_url = environment.redis.url();
    let first = PostgresRepository {
        configured_url: environment.configured_url.clone(),
        fixture: None,
        prefix: format!("oracle-scenario-first-{}", unique_id()?),
        redis_url: redis_url.clone(),
    };
    let second = PostgresRepository {
        configured_url: environment.configured_url.take(),
        fixture: None,
        prefix: format!("oracle-scenario-second-{}", unique_id()?),
        redis_url,
    };
    Ok((first, second))
}
