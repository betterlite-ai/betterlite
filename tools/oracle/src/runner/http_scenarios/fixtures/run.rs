use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use secrecy::SecretString;

use super::JournalPolling;
use super::PostgresRepository;
use super::SystemClock;
use super::gateway::GatewayPeerFactory;
use super::scripted::ScriptedProvider;
use crate::OracleError;
use crate::case::UpstreamAnswer;
use crate::cli::Options;
use crate::http;
use crate::http_scenarios::model::HttpScenario;
use crate::http_scenarios::run::{PeerFixture, ScenarioDependencies, run};
use crate::paths::unique_id;
use crate::runner::DatabaseEnvironment;

struct ScenarioEnvironment {
    provider: Arc<ScriptedProvider>,
    group_dir: PathBuf,
    configured_url: Option<SecretString>,
}

impl ScenarioEnvironment {
    fn start(root: &Path, scenario: &HttpScenario) -> Result<Self, OracleError> {
        let environment =
            <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])?;
        let answers = scenario
            .steps
            .iter()
            .flat_map(|step| step.upstream.iter().cloned())
            .collect::<Vec<UpstreamAnswer>>();
        let provider = Arc::new(ScriptedProvider::start(answers)?);
        let group_dir = root.join(format!("group-{}", unique_id()?));
        fs::create_dir_all(&group_dir).map_err(|source| OracleError::Filesystem {
            path: group_dir.clone(),
            source,
        })?;
        Ok(Self {
            provider,
            group_dir,
            configured_url: environment.postgres_test_url,
        })
    }
}

pub(crate) fn run_case(
    options: &Options,
    root: &Path,
    scenario: &HttpScenario,
) -> Result<bool, OracleError> {
    let mut environment = ScenarioEnvironment::start(root, scenario)?;
    let mut factory = GatewayPeerFactory {
        options,
        provider: Arc::clone(&environment.provider),
        client: http::client()?,
        group_dir: environment.group_dir.clone(),
        started: false,
    };
    let mut first_repository = PostgresRepository {
        configured_url: environment.configured_url.clone(),
        fixture: None,
        prefix: format!("oracle-scenario-first-{}", unique_id()?),
    };
    let mut second_repository = PostgresRepository {
        configured_url: environment.configured_url.take(),
        fixture: None,
        prefix: format!("oracle-scenario-second-{}", unique_id()?),
    };
    let mut first_polling = JournalPolling {
        path: environment.group_dir.join("redis-first.jsonl"),
    };
    let mut second_polling = JournalPolling {
        path: environment.group_dir.join("redis-second.jsonl"),
    };
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
    Ok(outcome.different)
}
