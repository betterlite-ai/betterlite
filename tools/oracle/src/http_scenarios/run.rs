use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::fixtures::http_scenarios::{ScenarioSetup, SetupJournalEntry};
use secrecy::{ExposeSecret as _, SecretString};

use self::peer::{PeerRun, check_deadline, expired, run_peer};
use super::artifacts::{self, ArtifactInput};
use super::compare::{self, Difference};
use super::error::ScenarioError;
use super::model::{CapturedValue, HttpScenario};
use crate::case::Request;
use crate::http::Response;
use crate::mock::ProviderRequest;

mod peer;

pub(crate) const DEADLINE: Duration = Duration::from_secs(120);

pub(crate) trait Clock {
    fn now(&self) -> Instant;
}

pub(crate) trait Peer {
    fn send(&mut self, request: &Request) -> Result<Response, ScenarioError>;
    fn provider_requests(&self) -> Result<Vec<ProviderRequest>, ScenarioError>;
    fn log(&self) -> Result<String, ScenarioError>;
    fn stop(&mut self) -> Result<(), ScenarioError>;
}

pub(crate) trait PeerFactory {
    fn start(&mut self, input: PeerStart<'_>) -> Result<Box<dyn Peer>, ScenarioError>;
}

pub(crate) struct PeerStart<'input> {
    pub(crate) role: PeerRole,
    pub(crate) config: &'input str,
    pub(crate) env: &'input BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerRole {
    First,
    Second,
}

impl PeerRole {
    pub(crate) const fn is_first(self) -> bool {
        match self {
            Self::First => true,
            Self::Second => false,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::First => "litellm",
            Self::Second => "betterlite",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PhysicalNames {
    pub(crate) database_url: SecretString,
    pub(crate) redis_url: String,
    pub(crate) redis_prefix: String,
}

pub(crate) trait FixtureRepository {
    fn provision(&mut self, need_database: bool) -> Result<PhysicalNames, ScenarioError>;
    fn insert(&mut self, setup: &ScenarioSetup) -> Result<Vec<SetupJournalEntry>, ScenarioError>;
    fn close(&mut self) -> Result<(), ScenarioError>;
}

pub(crate) trait PollingTransport {
    fn write(
        &mut self,
        prefix: &str,
        key: &str,
        value: &str,
        ttl_seconds: u64,
    ) -> Result<(), ScenarioError>;
}

pub(crate) struct PeerFixture<'input> {
    pub(crate) repository: &'input mut dyn FixtureRepository,
    pub(crate) polling: &'input mut dyn PollingTransport,
}

pub(crate) struct ScenarioDependencies<'input> {
    pub(crate) scenario: &'input HttpScenario,
    pub(crate) clock: &'input dyn Clock,
    pub(crate) factory: &'input mut dyn PeerFactory,
    pub(crate) first: PeerFixture<'input>,
    pub(crate) second: PeerFixture<'input>,
    pub(crate) upstream: &'input str,
    pub(crate) artifact_root: &'input Path,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScenarioOutcome {
    pub(crate) different: bool,
}

pub(crate) struct StepCapture {
    pub(crate) response: Response,
    pub(crate) provider: Vec<ProviderRequest>,
    pub(crate) provider_total: usize,
}

pub(crate) struct PeerTranscript {
    pub(crate) names: PhysicalNames,
    pub(crate) journal: Vec<SetupJournalEntry>,
    pub(crate) steps: Vec<StepCapture>,
    pub(crate) captures: BTreeMap<String, CapturedValue>,
    pub(crate) log: String,
}

struct ScenarioRun<'input> {
    scenario: &'input HttpScenario,
    clock: &'input dyn Clock,
    deadline: Instant,
    upstream: &'input str,
}

pub(crate) fn run(deps: ScenarioDependencies<'_>) -> Result<ScenarioOutcome, ScenarioError> {
    let ScenarioDependencies {
        scenario,
        clock,
        factory,
        first,
        second,
        upstream,
        artifact_root,
    } = deps;
    let deadline = clock
        .now()
        .checked_add(DEADLINE)
        .ok_or(ScenarioError::Deadline)?;
    let scenario_run = ScenarioRun {
        scenario,
        clock,
        deadline,
        upstream,
    };
    let first_transcript = run_peer(scenario_run.peer(PeerRole::First, first), factory)
        .map_err(|error| expired(clock, deadline, error))?;
    let second_transcript = run_peer(scenario_run.peer(PeerRole::Second, second), factory)
        .map_err(|error| expired(clock, deadline, error))?;
    scenario_run.finish(artifact_root, &first_transcript, &second_transcript)
}

impl<'input> ScenarioRun<'input> {
    fn peer(&self, role: PeerRole, fixture: PeerFixture<'input>) -> PeerRun<'input> {
        PeerRun {
            role,
            scenario: self.scenario,
            clock: self.clock,
            deadline: self.deadline,
            upstream: self.upstream,
            fixture,
            peer: None,
            captures: BTreeMap::new(),
        }
    }

    fn finish(
        &self,
        artifact_root: &Path,
        first: &PeerTranscript,
        second: &PeerTranscript,
    ) -> Result<ScenarioOutcome, ScenarioError> {
        check_deadline(self.clock, self.deadline)?;
        ensure_isolated(&first.names, &second.names)?;
        ensure_journal(&first.journal, &second.journal)?;
        let difference =
            compare::compare_transcripts(&first.steps, &second.steps, &self.scenario.steps);
        let secrets = collect_secrets([first, second]);
        let directory = artifacts::write(ArtifactInput {
            root: artifact_root,
            scenario: self.scenario,
            first,
            second,
            difference: difference.as_ref().err(),
            secrets: &secrets,
        })?;
        report(self.scenario, &directory, difference.as_ref().err())?;
        check_deadline(self.clock, self.deadline)?;
        Ok(ScenarioOutcome {
            different: difference.is_err(),
        })
    }
}

fn report(
    scenario: &HttpScenario,
    directory: &Path,
    difference: Option<&Difference>,
) -> Result<(), ScenarioError> {
    let outcome = difference.map_or_else(
        || format!("PASS {}: {}", scenario.name, directory.display()),
        |error| format!("FAIL {}: {}", scenario.name, error.pointer),
    );
    writeln!(io::stdout().lock(), "{outcome}").map_err(ScenarioError::Output)?;
    Ok(())
}

fn ensure_isolated(first: &PhysicalNames, second: &PhysicalNames) -> Result<(), ScenarioError> {
    let first_url = first.database_url.expose_secret();
    let second_url = second.database_url.expose_secret();
    if !first_url.is_empty() && first_url == second_url {
        return Err(ScenarioError::SetupMismatch {
            message: "both peers share one postgres namespace".to_owned(),
        });
    }
    if first.redis_prefix == second.redis_prefix {
        return Err(ScenarioError::SetupMismatch {
            message: "both peers share one redis prefix".to_owned(),
        });
    }
    Ok(())
}

fn ensure_journal(
    first: &[SetupJournalEntry],
    second: &[SetupJournalEntry],
) -> Result<(), ScenarioError> {
    if first == second {
        return Ok(());
    }
    Err(ScenarioError::SetupMismatch {
        message: "the setup journals differ".to_owned(),
    })
}

fn collect_secrets(transcripts: [&PeerTranscript; 2]) -> Vec<SecretString> {
    let mut secrets = Vec::new();
    for transcript in transcripts {
        secrets.push(transcript.names.database_url.clone());
        secrets.push(SecretString::from(transcript.names.redis_prefix.clone()));
        for value in transcript.captures.values().filter(|value| value.secret) {
            secrets.push(value.text.clone());
        }
    }
    secrets.retain(|secret| !secret.expose_secret().is_empty());
    secrets
}

pub(super) fn render_config(
    scenario: &HttpScenario,
    names: &PhysicalNames,
    upstream: &str,
) -> String {
    scenario
        .config
        .replace("{{upstream}}", upstream)
        .replace("{{postgres}}", names.database_url.expose_secret())
        .replace("{{redis}}", &names.redis_url)
}

pub(super) fn child_environment(caller: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env = caller.clone();
    env.insert("LITELLM_LOCAL_MODEL_COST_MAP".to_owned(), "True".to_owned());
    env.insert("LITELLM_MASTER_KEY".to_owned(), "oracle-master".to_owned());
    env
}
