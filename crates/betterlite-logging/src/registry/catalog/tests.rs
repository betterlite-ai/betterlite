#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::callback_settings::names::CallbackName;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{BaseLogger, CatalogError, CatalogInputs, LoggerCatalog};
use crate::callback_config::{CallbackVars, FeatureFlags};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::resolver::LoggerResolver;
use crate::registry::error::RegistryError;
use crate::registry::phases::{CallbackList, CallbackPhase};
use crate::registry::{CallbackRegistry, Registration};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let result = with_default(subscriber, action);
    (result, buffer.contents())
}

fn inputs() -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    CatalogInputs {
        config: Arc::new(
            ConfigLoader {
                lookup: BTreeMap::new(),
            }
            .load("")
            .expect("valid config"),
        ),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(BTreeMap::<String, String>::new()),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn catalog() -> LoggerCatalog {
    LoggerCatalog::new(inputs())
}

fn counting_catalog(builds: &Arc<AtomicUsize>) -> LoggerCatalog {
    let counter = Arc::clone(builds);
    LoggerCatalog {
        inputs: inputs(),
        build: Arc::new(move |callback: CallbackName, _inputs: &CatalogInputs| {
            counter.fetch_add(1, Ordering::SeqCst);
            let logger: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
                name: callback.as_str().to_owned(),
            });
            Ok(logger)
        }),
        built: Mutex::new(BTreeMap::new()),
    }
}

fn flags() -> FeatureFlags {
    FeatureFlags {
        otel_v2_enabled: false,
    }
}

fn failing_builder(
    _callback: CallbackName,
    _inputs: &CatalogInputs,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    Err(CatalogError::Init {
        message: "arize needs ARIZE_API_KEY".to_owned(),
    })
}

#[rstest]
#[case::unknown_name("not_a_callback")]
#[case::stub_name("langfuse")]
fn an_unknown_or_unbuilt_name_fails_naming_the_callback(#[case] name: &str) {
    assert!(matches!(
        catalog().resolve_logger(name),
        Err(RegistryError::UnavailableAdapter { name: rejected }) if rejected == name
    ));
}

#[rstest]
#[case::prometheus("prometheus")]
#[case::datadog("datadog")]
#[case::litellm_agent("litellm_agent")]
#[case::generic_api("generic_api")]
#[case::sample_callback("sample_callback")]
#[case::rubrik("rubrik")]
#[case::sumologic("sumologic")]
#[case::qualifire_eval("qualifire_eval")]
#[case::ramp("ramp")]
#[case::dynamic_rate_limiter("dynamic_rate_limiter")]
#[case::dynamic_rate_limiter_v3("dynamic_rate_limiter_v3")]
#[case::anthropic_cache_control_hook("anthropic_cache_control_hook")]
#[case::vector_store_pre_call_hook("vector_store_pre_call_hook")]
#[case::compression_interception("compression_interception")]
fn a_server_or_hook_name_is_refused(#[case] name: &str) {
    assert!(matches!(
        catalog().resolve_logger(name),
        Err(RegistryError::UnavailableAdapter { name: rejected }) if rejected == name
    ));
}

#[rstest]
#[case::pagerduty("pagerduty")]
#[case::resend_email("resend_email")]
#[case::sendgrid_email("sendgrid_email")]
#[case::smtp_email("smtp_email")]
fn an_enterprise_factory_name_builds_a_base_logger(#[case] name: &str) {
    let logger = catalog()
        .resolve_logger(name)
        .expect("the name is available")
        .expect("the factory builds a logger");
    assert_eq!(logger.callback_name(), name);
}

#[test]
fn a_logger_is_built_once_and_reused() {
    let builds = Arc::new(AtomicUsize::new(0));
    let catalog = counting_catalog(&builds);
    assert!(catalog.existing("pagerduty").is_none());
    let first = catalog
        .resolve_logger("pagerduty")
        .expect("the name is available")
        .expect("the factory builds a logger");
    let second = catalog
        .resolve_logger("pagerduty")
        .expect("the name is available")
        .expect("the second resolve reuses it");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    let existing = catalog.existing("pagerduty").expect("the logger is stored");
    assert!(Arc::ptr_eq(&first, &existing));
    assert!(catalog.existing("langsmith").is_none());
}

#[test]
fn storing_a_second_logger_for_a_name_keeps_the_first() {
    let catalog = catalog();
    let first: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "first".to_owned(),
    });
    let second: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "second".to_owned(),
    });
    let stored_first = catalog.store(CallbackName::Pagerduty, Arc::clone(&first));
    let stored_second = catalog.store(CallbackName::Pagerduty, Arc::clone(&second));
    assert!(Arc::ptr_eq(&stored_first, &first));
    assert!(Arc::ptr_eq(&stored_second, &first));
}

#[test]
fn an_init_error_gives_none_and_logs_once() {
    let catalog = catalog();
    let (result, log) = logged(|| {
        let built = failing_builder(CallbackName::Arize, &catalog.inputs);
        catalog.resolve_built(CallbackName::Arize, built)
    });
    assert!(
        result
            .expect("an init error is not a resolve error")
            .is_none()
    );
    let line = log
        .lines()
        .find(|line| line.contains("Non-Blocking Error"))
        .expect("the init error is logged");
    assert!(line.contains("ERROR"));
    assert!(line.contains(
        "[Non-Blocking Error] Error initializing custom logger: arize needs ARIZE_API_KEY"
    ));
    assert_eq!(
        log.matches("[Non-Blocking Error] Error initializing custom logger")
            .count(),
        1
    );
}

#[test]
fn the_logger_resolver_ignores_request_params() {
    let catalog = catalog();
    let resolver: &dyn LoggerResolver = &catalog;
    let params = BTreeMap::from([("dd_api_key".to_owned(), "secret".to_owned())]);
    let logger = resolver
        .resolve("pagerduty", &params)
        .expect("the name is available")
        .expect("the factory builds a logger");
    assert_eq!(logger.callback_name(), "pagerduty");
}

#[test]
fn a_registry_registers_the_catalog_logger_for_success() {
    let resolver: Arc<dyn LoggerResolver> = Arc::new(catalog());
    let mut registry = CallbackRegistry::new(100).with_resolver(resolver);
    assert_eq!(
        registry
            .register_configured(
                "pagerduty",
                &CallbackVars::new(),
                CallbackList::Success,
                flags()
            )
            .expect("pagerduty is available"),
        Registration::Registered
    );
    let loggers = registry.loggers(CallbackPhase::Success);
    assert_eq!(loggers.len(), 1);
    assert_eq!(
        loggers.first().expect("one logger").callback_name(),
        "pagerduty"
    );
}

#[test]
fn a_registry_with_the_catalog_refuses_an_unbuilt_name() {
    let resolver: Arc<dyn LoggerResolver> = Arc::new(catalog());
    let mut registry = CallbackRegistry::new(100).with_resolver(resolver);
    assert!(matches!(
        registry.register_configured(
            "langfuse",
            &CallbackVars::new(),
            CallbackList::Success,
            flags()
        ),
        Err(RegistryError::UnavailableAdapter { name }) if name == "langfuse"
    ));
    assert!(registry.list_identities(CallbackList::Success).is_empty());
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
}
