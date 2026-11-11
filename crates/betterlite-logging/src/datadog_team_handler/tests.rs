#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::ExposeSecret;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::datadog_logging::clock::Clock;
use crate::datadog_logging::error::DatadogError;
use crate::datadog_logging::logger::{DatadogDeps, DatadogLogger, uuid_id};
use crate::datadog_logging::settings::DatadogInitParams;
use crate::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use crate::dynamic_logger_cache::DynamicLoggerCache;

use super::{
    DatadogTeamHandlerError, DatadogTeamSettings, datadog_credentials, datadog_credentials_passed,
    get_datadog_logger_for_request,
};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

struct AcceptingTransport;

#[async_trait]
impl Transport for AcceptingTransport {
    async fn post(&self, _request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        Ok(TransportResponse {
            status: 202,
            body: Vec::new(),
        })
    }
}

struct CountingClock {
    builds: Arc<AtomicUsize>,
}

impl Clock for CountingClock {
    fn now_unix(&self) -> Duration {
        self.builds.fetch_add(1, Ordering::SeqCst);
        Duration::ZERO
    }
}

fn deps(builds: &Arc<AtomicUsize>) -> DatadogDeps {
    DatadogDeps {
        transport: Arc::new(AcceptingTransport),
        clock: Arc::new(CountingClock {
            builds: Arc::clone(builds),
        }),
        ids: uuid_id,
        tasks: None,
    }
}

fn cache() -> DynamicLoggerCache<Arc<DatadogLogger>> {
    DynamicLoggerCache::new(
        Arc::new(|_logger: Arc<DatadogLogger>| {}),
        Arc::new(|| 0.0_f64),
    )
}

fn baseline_settings() -> DatadogTeamSettings {
    DatadogTeamSettings {
        params: DatadogInitParams {
            turn_off_message_logging: false,
        },
        use_v1: false,
    }
}

fn get(
    params: &BTreeMap<String, String>,
    cache: &DynamicLoggerCache<Arc<DatadogLogger>>,
    environment: &BTreeMap<String, String>,
    builds: &Arc<AtomicUsize>,
) -> Result<Arc<DatadogLogger>, DatadogTeamHandlerError> {
    get_datadog_logger_for_request(
        params,
        cache,
        environment,
        deps(builds),
        baseline_settings(),
    )
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .expect("the capture lock is not poisoned")
            .clone();
        String::from_utf8(bytes).expect("the captured output is UTF-8")
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the capture lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug() -> (CapturedLogs, DefaultGuard) {
    let captured = CapturedLogs::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

#[test]
fn credentials_read_the_four_dynamic_parameters() {
    let params = map(&[
        ("dd_api_key", "team-key"),
        ("dd_site", "us5.datadoghq.com"),
        ("dd_agent_host", "agent.local"),
        ("dd_agent_port", "10519"),
    ]);
    let credentials = datadog_credentials(&params);
    assert_eq!(credentials.dd_api_key.as_deref(), Some("team-key"));
    assert_eq!(credentials.dd_site.as_deref(), Some("us5.datadoghq.com"));
    assert_eq!(credentials.dd_agent_host.as_deref(), Some("agent.local"));
    assert_eq!(credentials.dd_agent_port.as_deref(), Some("10519"));
}

#[test]
fn credentials_leave_absent_parameters_as_none() {
    let credentials = datadog_credentials(&map(&[]));
    assert!(credentials.dd_api_key.is_none());
    assert!(credentials.dd_site.is_none());
    assert!(credentials.dd_agent_host.is_none());
    assert!(credentials.dd_agent_port.is_none());
}

#[rstest]
#[case::api_key("dd_api_key")]
#[case::site("dd_site")]
#[case::agent_host("dd_agent_host")]
fn credentials_passed_counts_each_team_credential_even_when_empty(#[case] name: &str) {
    assert!(datadog_credentials_passed(&map(&[(name, "")])));
}

#[test]
fn credentials_passed_ignores_a_port_alone() {
    assert!(!datadog_credentials_passed(&map(&[(
        "dd_agent_port",
        "10518"
    )])));
}

#[test]
fn credentials_passed_is_false_without_any_credential() {
    assert!(!datadog_credentials_passed(&map(&[])));
}

#[test]
fn the_same_credentials_return_the_same_cached_logger() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[("dd_api_key", "team-key"), ("dd_site", "us5.datadoghq.com")]);
    let environment = map(&[]);
    let first = get(&params, &cache, &environment, &builds).expect("the first logger");
    let second = get(&params, &cache, &environment, &builds).expect("the cached logger");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(builds.load(Ordering::SeqCst), 1);
}

#[test]
fn a_different_credential_set_builds_a_second_logger() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let environment = map(&[]);
    let first = get(
        &map(&[("dd_api_key", "team-a"), ("dd_site", "us5.datadoghq.com")]),
        &cache,
        &environment,
        &builds,
    )
    .expect("the first team logger");
    let second = get(
        &map(&[("dd_api_key", "team-b"), ("dd_site", "us5.datadoghq.com")]),
        &cache,
        &environment,
        &builds,
    )
    .expect("the second team logger");
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(builds.load(Ordering::SeqCst), 2);
}

#[test]
fn a_different_agent_port_builds_a_second_logger() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let environment = map(&[]);
    let first = get(
        &map(&[
            ("dd_api_key", "team-key"),
            ("dd_agent_host", "agent.local"),
            ("dd_agent_port", "10518"),
        ]),
        &cache,
        &environment,
        &builds,
    )
    .expect("the first team logger");
    let second = get(
        &map(&[
            ("dd_api_key", "team-key"),
            ("dd_agent_host", "agent.local"),
            ("dd_agent_port", "10519"),
        ]),
        &cache,
        &environment,
        &builds,
    )
    .expect("the second team logger");
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(builds.load(Ordering::SeqCst), 2);
}

#[test]
fn an_absent_credential_and_an_empty_one_are_different_cache_keys() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let environment = map(&[]);
    let absent = map(&[("dd_agent_host", "agent.local")]);
    let empty = map(&[("dd_agent_host", "agent.local"), ("dd_api_key", "")]);
    let absent_logger = get(&absent, &cache, &environment, &builds).expect("the absent-key logger");
    let empty_logger = get(&empty, &cache, &environment, &builds).expect("the empty-key logger");
    assert!(!Arc::ptr_eq(&absent_logger, &empty_logger));
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert!(absent_logger.settings.api_key.is_none());
    assert_eq!(
        empty_logger
            .settings
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("")
    );
}

#[test]
fn a_caller_chosen_agent_host_never_gets_the_proxy_api_key() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[("dd_agent_host", "agent.local")]);
    let environment = map(&[("DD_API_KEY", "proxy-key")]);
    let logger = get(&params, &cache, &environment, &builds).expect("the team logger");
    assert!(logger.settings.api_key.is_none());
    assert_eq!(
        logger.settings.intake_url,
        "http://agent.local:10518/api/v2/logs"
    );
}

#[test]
fn a_caller_chosen_site_never_gets_the_proxy_api_key() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[("dd_site", "us5.datadoghq.com")]);
    let environment = map(&[("DD_API_KEY", "proxy-key")]);
    let result = get(&params, &cache, &environment, &builds);
    assert!(matches!(
        result,
        Err(DatadogTeamHandlerError::Build(DatadogError::MissingApiKey))
    ));
}

#[test]
fn a_team_site_with_its_own_key_builds_with_that_key() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[("dd_api_key", "team-key"), ("dd_site", "us5.datadoghq.com")]);
    let environment = map(&[("DD_API_KEY", "proxy-key")]);
    let logger = get(&params, &cache, &environment, &builds).expect("the team logger");
    assert_eq!(
        logger
            .settings
            .api_key
            .as_ref()
            .expect("the team key")
            .expose_secret(),
        "team-key"
    );
    assert_eq!(
        logger.settings.intake_url,
        "https://http-intake.logs.us5.datadoghq.com/api/v2/logs"
    );
}

#[test]
fn a_team_api_key_alone_falls_back_to_the_proxy_site() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[("dd_api_key", "team-key")]);
    let environment = map(&[
        ("DD_API_KEY", "proxy-key"),
        ("DD_SITE", "env-site.datadoghq.com"),
    ]);
    let logger = get(&params, &cache, &environment, &builds).expect("the team logger");
    assert_eq!(
        logger
            .settings
            .api_key
            .as_ref()
            .expect("the team key")
            .expose_secret(),
        "team-key"
    );
    assert_eq!(
        logger.settings.intake_url,
        "https://http-intake.logs.env-site.datadoghq.com/api/v2/logs"
    );
}

#[test]
fn a_request_without_a_destination_uses_the_proxy_credentials() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[]);
    let environment = map(&[
        ("DD_API_KEY", "proxy-key"),
        ("DD_SITE", "proxy-site.datadoghq.com"),
    ]);
    let logger = get(&params, &cache, &environment, &builds).expect("the proxy logger");
    assert_eq!(
        logger
            .settings
            .api_key
            .as_ref()
            .expect("the proxy key")
            .expose_secret(),
        "proxy-key"
    );
    assert_eq!(
        logger.settings.intake_url,
        "https://http-intake.logs.proxy-site.datadoghq.com/api/v2/logs"
    );
}

#[test]
fn the_agent_host_and_port_are_passed_as_given() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[
        ("dd_api_key", "team-key"),
        ("dd_agent_host", "agent.local"),
        ("dd_agent_port", "10519"),
    ]);
    let environment = map(&[]);
    let logger = get(&params, &cache, &environment, &builds).expect("the team logger");
    assert_eq!(
        logger.settings.intake_url,
        "http://agent.local:10519/api/v2/logs"
    );
}

#[test]
fn the_team_logger_uses_the_proxy_datadog_settings() {
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[("dd_api_key", "team-key"), ("dd_site", "us5.datadoghq.com")]);
    let environment = map(&[]);
    let settings = DatadogTeamSettings {
        params: DatadogInitParams {
            turn_off_message_logging: true,
        },
        use_v1: true,
    };
    let logger =
        get_datadog_logger_for_request(&params, &cache, &environment, deps(&builds), settings)
            .expect("the team logger");
    assert!(logger.settings.use_v1);
    assert!(logger.settings.options.turn_off_message_logging);
}

#[test]
fn a_new_logger_logs_the_team_scope_without_the_credentials() {
    let (captured, _guard) = capture_debug();
    let cache = cache();
    let builds = Arc::new(AtomicUsize::new(0));
    let params = map(&[
        ("dd_api_key", "team-secret-key"),
        ("dd_site", "us5.datadoghq.com"),
    ]);
    let environment = map(&[]);
    let logger = get(&params, &cache, &environment, &builds).expect("the team logger");
    let text = captured.text();
    assert!(
        text.contains("Datadog: Created and cached new DataDogLogger for team-scoped credentials")
    );
    assert!(!text.contains("team-secret-key"));
    assert_eq!(
        logger
            .settings
            .api_key
            .as_ref()
            .expect("the team key")
            .expose_secret(),
        "team-secret-key"
    );
}
