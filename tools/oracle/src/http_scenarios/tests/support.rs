use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::fixtures::http_scenarios::{
    ScenarioSetup, SetupJournalEntry, SetupList,
};
use secrecy::SecretString;
use serde_json::Value;

use crate::case::{BodyKind, Request};
use crate::http::Response;
use crate::http_scenarios::decode_http_scenario;
use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::model::{HttpScenario, Limits, ScenarioRequest, Step, StepCompare};
use crate::http_scenarios::run::{
    Clock, FixtureRepository, Peer, PeerFactory, PeerFixture, PeerStart, PhysicalNames,
    PollingTransport, ScenarioDependencies, StepCapture, run,
};
use crate::mock::ProviderRequest;

#[derive(Clone)]
pub(crate) struct FakeClock {
    start: Instant,
    offset: Arc<Mutex<Duration>>,
}

impl FakeClock {
    pub(crate) fn new() -> Self {
        Self {
            start: Instant::now(),
            offset: Arc::new(Mutex::new(Duration::ZERO)),
        }
    }

    pub(crate) fn handle(&self) -> Arc<Mutex<Duration>> {
        Arc::clone(&self.offset)
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.start
            .checked_add(*self.offset.lock().unwrap())
            .unwrap()
    }
}

pub(crate) struct PlannedPeer {
    responses: VecDeque<Result<Response, String>>,
    provider_per_send: VecDeque<Vec<ProviderRequest>>,
    advance: Option<(Arc<Mutex<Duration>>, Duration)>,
    requests: Arc<Mutex<Vec<Request>>>,
    provider: Arc<Mutex<Vec<ProviderRequest>>>,
    log: String,
    stopped: Arc<AtomicUsize>,
}

impl PlannedPeer {
    pub(crate) fn new(responses: Vec<Result<Response, String>>) -> Self {
        Self {
            responses: VecDeque::from(responses),
            provider_per_send: VecDeque::new(),
            advance: None,
            requests: Arc::new(Mutex::new(Vec::new())),
            provider: Arc::new(Mutex::new(Vec::new())),
            log: String::new(),
            stopped: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn with_provider(mut self, per_send: Vec<Vec<ProviderRequest>>) -> Self {
        self.provider_per_send = VecDeque::from(per_send);
        self
    }

    pub(crate) fn advancing(mut self, offset: Arc<Mutex<Duration>>, duration: Duration) -> Self {
        self.advance = Some((offset, duration));
        self
    }

    pub(crate) fn with_log(mut self, log: &str) -> Self {
        self.log = log.to_owned();
        self
    }

    pub(crate) fn requests(&self) -> Arc<Mutex<Vec<Request>>> {
        Arc::clone(&self.requests)
    }

    pub(crate) fn stopped(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.stopped)
    }
}

impl Peer for PlannedPeer {
    fn send(&mut self, request: &Request) -> Result<Response, ScenarioError> {
        locked(&self.requests)?.push(request.clone());
        if let Some((offset, duration)) = &self.advance {
            let mut current = locked(offset)?;
            *current = current.saturating_add(*duration);
        }
        if let Some(requests) = self.provider_per_send.pop_front() {
            locked(&self.provider)?.extend(requests);
        }
        match self.responses.pop_front() {
            Some(Ok(response)) => Ok(response),
            Some(Err(message)) => Err(ScenarioError::Io(io::Error::other(message))),
            None => Err(ScenarioError::Io(io::Error::other("no scripted response"))),
        }
    }

    fn provider_requests(&self) -> Result<Vec<ProviderRequest>, ScenarioError> {
        Ok(locked(&self.provider)?.clone())
    }

    fn log(&self) -> Result<String, ScenarioError> {
        Ok(self.log.clone())
    }

    fn stop(&mut self) -> Result<(), ScenarioError> {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(crate) struct StartRecord {
    pub(crate) config: String,
    pub(crate) env: BTreeMap<String, String>,
}

#[derive(Default)]
pub(crate) struct FakeFactory {
    peers: VecDeque<PlannedPeer>,
    starts: Vec<StartRecord>,
}

impl FakeFactory {
    pub(crate) fn push(&mut self, peer: PlannedPeer) {
        self.peers.push_back(peer);
    }

    pub(crate) fn starts(&self) -> &[StartRecord] {
        &self.starts
    }
}

impl PeerFactory for FakeFactory {
    fn start(&mut self, input: PeerStart<'_>) -> Result<Box<dyn Peer>, ScenarioError> {
        self.starts.push(StartRecord {
            config: input.config.to_owned(),
            env: input.env.clone(),
        });
        self.peers.pop_front().map_or_else(
            || Err(ScenarioError::Io(io::Error::other("no scripted peer"))),
            |peer| {
                let boxed: Box<dyn Peer> = Box::new(peer);
                Ok(boxed)
            },
        )
    }
}

pub(crate) struct FakeFixture {
    pub(crate) names: PhysicalNames,
    pub(crate) needs: Vec<bool>,
    pub(crate) setup: Option<ScenarioSetup>,
    pub(crate) journal: Vec<SetupJournalEntry>,
    pub(crate) closes: usize,
}

impl FakeFixture {
    pub(crate) fn new(names: PhysicalNames, journal: Vec<SetupJournalEntry>) -> Self {
        Self {
            names,
            needs: Vec::new(),
            setup: None,
            journal,
            closes: 0,
        }
    }
}

impl FixtureRepository for FakeFixture {
    fn provision(&mut self, need_database: bool) -> Result<PhysicalNames, ScenarioError> {
        self.needs.push(need_database);
        Ok(self.names.clone())
    }

    fn insert(&mut self, setup: &ScenarioSetup) -> Result<Vec<SetupJournalEntry>, ScenarioError> {
        self.setup = Some(setup.clone());
        Ok(self.journal.clone())
    }

    fn close(&mut self) -> Result<(), ScenarioError> {
        self.closes = self.closes.saturating_add(1);
        Ok(())
    }
}

pub(crate) struct PollWrite {
    pub(crate) prefix: String,
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) ttl_seconds: u64,
}

#[derive(Default)]
pub(crate) struct FakePolling {
    pub(crate) writes: Vec<PollWrite>,
    pub(crate) fail: bool,
}

impl PollingTransport for FakePolling {
    fn write(
        &mut self,
        prefix: &str,
        key: &str,
        value: &str,
        ttl_seconds: u64,
    ) -> Result<(), ScenarioError> {
        if self.fail {
            return Err(ScenarioError::Io(io::Error::other(
                "the polling transport is unavailable",
            )));
        }
        self.writes.push(PollWrite {
            prefix: prefix.to_owned(),
            key: key.to_owned(),
            value: value.to_owned(),
            ttl_seconds,
        });
        Ok(())
    }
}

pub(crate) fn physical_names(label: &str) -> PhysicalNames {
    PhysicalNames {
        database_url: SecretString::from(format!("postgres://{label}")),
        redis_url: format!("redis://{label}/0"),
        redis_prefix: format!("prefix-{label}"),
    }
}

pub(crate) struct Harness {
    pub(crate) scenario: HttpScenario,
    pub(crate) factory: FakeFactory,
    pub(crate) first: FakeFixture,
    pub(crate) second: FakeFixture,
    pub(crate) first_polling: FakePolling,
    pub(crate) second_polling: FakePolling,
    pub(crate) clock: FakeClock,
    pub(crate) upstream: String,
    pub(crate) root: tempfile::TempDir,
}

impl Harness {
    pub(crate) fn new(text: &str) -> Self {
        Self::with_limits(text, &Limits::default())
    }

    pub(crate) fn with_limits(text: &str, limits: &Limits) -> Self {
        Self {
            scenario: decode_http_scenario(text.as_bytes(), limits).unwrap(),
            factory: FakeFactory::default(),
            first: FakeFixture::new(physical_names("first"), Vec::new()),
            second: FakeFixture::new(physical_names("second"), Vec::new()),
            first_polling: FakePolling::default(),
            second_polling: FakePolling::default(),
            clock: FakeClock::new(),
            upstream: "http://upstream".to_owned(),
            root: tempfile::tempdir().unwrap(),
        }
    }

    pub(crate) fn push_peer(&mut self, peer: PlannedPeer) {
        self.factory.push(peer);
    }

    pub(crate) fn start(&mut self) -> Result<bool, ScenarioError> {
        let dependencies = self.dependencies();
        run(dependencies)
    }

    fn dependencies(&mut self) -> ScenarioDependencies<'_> {
        ScenarioDependencies {
            scenario: &self.scenario,
            clock: &self.clock,
            factory: &mut self.factory,
            first: PeerFixture {
                repository: &mut self.first,
                polling: &mut self.first_polling,
            },
            second: PeerFixture {
                repository: &mut self.second,
                polling: &mut self.second_polling,
            },
            upstream: &self.upstream,
            artifact_root: self.root.path(),
        }
    }
}

pub(crate) fn response(status: u16, body: Value) -> Response {
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    Response {
        status,
        headers,
        body,
    }
}

fn locked<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, ScenarioError> {
    mutex
        .lock()
        .map_err(|_source| ScenarioError::Io(io::Error::other("the test mutex was poisoned")))
}

pub(crate) fn artifact_files(root: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files
}

fn collect_files(directory: &Path, files: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push((path.clone(), fs::read_to_string(&path).unwrap()));
        }
    }
}

pub(crate) fn artifacts_contain(root: &Path, needle: &str) -> bool {
    artifact_files(root)
        .iter()
        .any(|(_, text)| text.contains(needle))
}

pub(crate) fn artifact_named(root: &Path, name: &str) -> String {
    artifact_files(root)
        .into_iter()
        .find(|(path, _)| path.file_name().is_some_and(|stem| stem == name))
        .map(|(_, text)| text)
        .unwrap()
}

pub(crate) fn expected_journal(setup: &ScenarioSetup) -> Vec<SetupJournalEntry> {
    [
        (SetupList::Users, setup.users.len()),
        (SetupList::Teams, setup.teams.len()),
        (SetupList::Memberships, setup.memberships.len()),
        (SetupList::Budgets, setup.budgets.len()),
        (SetupList::JwtMappings, setup.jwt_mappings.len()),
        (SetupList::Keys, setup.keys.len()),
        (SetupList::ManagedFiles, setup.managed_files.len()),
        (SetupList::ManagedObjects, setup.managed_objects.len()),
        (SetupList::VectorStores, setup.vector_stores.len()),
    ]
    .into_iter()
    .flat_map(|(list, count)| (0..count).map(move |index| SetupJournalEntry { list, index }))
    .collect()
}

pub(crate) fn journal_list_order(journal: &[SetupJournalEntry]) -> Vec<SetupList> {
    let mut order = Vec::new();
    for entry in journal {
        if order.last() != Some(&entry.list) {
            order.push(entry.list);
        }
    }
    order
}

pub(crate) fn step_yaml(index: usize) -> String {
    format!("  - name: step-{index}\n    request:\n      method: GET\n      path: /step-{index}\n")
}

pub(crate) fn steps_yaml(count: usize) -> String {
    (0..count).map(step_yaml).collect()
}

pub(crate) fn scenario_text(steps: &str) -> String {
    format!(
        "kind: http_scenario\nname: scenario-test\nconfig: |-\n  model_list: []\nenv: {{}}\nsteps:\n{steps}"
    )
}

pub(crate) fn provider_request(method: &str, path: &str, body: Value) -> ProviderRequest {
    ProviderRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        headers: BTreeMap::new(),
        body,
    }
}

pub(crate) fn response_with_headers(
    status: u16,
    headers: BTreeMap<String, String>,
    body: Value,
) -> Response {
    Response {
        status,
        headers,
        body,
    }
}

pub(crate) fn capture(status: u16, body: Value) -> StepCapture {
    StepCapture {
        response: response(status, body),
        provider: Vec::new(),
        provider_total: 0,
    }
}

pub(crate) fn capture_with_provider(body: Value, provider: Vec<ProviderRequest>) -> StepCapture {
    StepCapture {
        provider_total: provider.len(),
        provider,
        response: response(200, body),
    }
}

pub(crate) fn step_with_compare(compare: StepCompare) -> Step {
    Step {
        name: "step".to_owned(),
        request: ScenarioRequest {
            method: "GET".to_owned(),
            path: "/step".to_owned(),
            headers: BTreeMap::new(),
            body: Value::Null,
            body_kind: BodyKind::Json,
        },
        upstream: Vec::new(),
        capture: Vec::new(),
        compare,
    }
}

pub(crate) fn compare_with_headers(
    headers: Vec<String>,
    headers_present: Vec<String>,
) -> StepCompare {
    StepCompare {
        headers,
        headers_present,
        upstream_headers: Vec::new(),
        ignore: Vec::new(),
    }
}

pub(crate) fn two_step_scenario() -> String {
    scenario_text(&steps_yaml(2))
}
