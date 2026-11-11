use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::rules::RuleDecision;
use betterlite_core::rules::RuleEvaluator;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::logging_state::correlation::CorrelationSettings;
use betterlite_logging::logging_state::correlation::SecretRedaction;
use betterlite_logging::registry::error::RegistryError;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::MINIMUM_CUSTOM_KEY_LENGTH;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use chrono::FixedOffset;
use indexmap::IndexMap;

use crate::call_setup::RequestMap;
use crate::call_setup::callbacks::CallbackLists;
use crate::call_setup::callbacks::CustomLoggers;
use crate::call_setup::credentials::Credential;
use crate::call_setup::credentials::CredentialLookup;
use crate::call_setup::function_setup::BreadcrumbSink;
use crate::call_setup::function_setup::CallSetup;
use crate::call_setup::function_setup::CallSetupDependencies;
use crate::call_setup::function_setup::CallSetupError;
use crate::call_setup::function_setup::TraceSource;
use crate::call_setup::function_setup::function_setup;
use crate::call_setup::hooks::DeploymentHook;
use crate::call_setup::hooks::Guardrail;
use crate::call_setup::providers::ProviderRegistry;

pub(crate) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn request(source: &str) -> RequestMap {
    serde_json::from_str(source).expect("the fixture request is a JSON object")
}

pub(crate) fn value(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture value is JSON")
}

pub(crate) fn chat_request(extra: &str) -> RequestMap {
    let mut base = request(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"litellm_call_id":"call-1","litellm_trace_id":"trace-1"}"#,
    );
    for (key, value) in request(extra) {
        base.insert(key, value);
    }
    base
}

pub(crate) struct Marker {
    pub(crate) label: String,
    pub(crate) guardrail: Option<Guardrail>,
}

#[async_trait]
impl CallbackLogger for Marker {
    fn callback_name(&self) -> &str {
        &self.label
    }
}

#[async_trait]
impl DeploymentHook for Marker {
    fn guardrail(&self) -> Option<Guardrail> {
        self.guardrail.clone()
    }
}

pub(crate) enum Outcome {
    Logger(String),
    Missing,
}

#[derive(Default)]
pub(crate) struct Resolver {
    outcomes: Mutex<BTreeMap<String, Outcome>>,
    markers: Mutex<BTreeMap<String, Arc<Marker>>>,
}

impl Resolver {
    pub(crate) fn with(outcomes: &[(&str, Outcome)]) -> Self {
        let mut map = BTreeMap::new();
        for (name, outcome) in outcomes {
            match outcome {
                Outcome::Logger(label) => {
                    map.insert((*name).to_owned(), Outcome::Logger(label.clone()));
                }
                Outcome::Missing => {
                    map.insert((*name).to_owned(), Outcome::Missing);
                }
            }
        }
        Self {
            outcomes: Mutex::new(map),
            markers: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn known(&self, name: &str) -> bool {
        self.outcomes
            .lock()
            .expect("the resolver lock is not poisoned")
            .contains_key(name)
    }

    pub(crate) fn marker(&self, label: &str) -> Arc<Marker> {
        let mut markers = self.markers.lock().expect("the marker lock is free");
        Arc::clone(
            markers
                .entry(label.to_owned())
                .or_insert_with(|| Arc::new(plain_marker(label))),
        )
    }
}

impl LoggerResolver for Resolver {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        let outcomes =
            self.outcomes
                .lock()
                .map_err(|_poison| RegistryError::UnavailableAdapter {
                    name: name.to_owned(),
                })?;
        match outcomes.get(name) {
            Some(Outcome::Logger(label)) => {
                let resolved = label.clone();
                Ok(Some(self.marker(&resolved)))
            }
            Some(Outcome::Missing) => Ok(None),
            None => Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            }),
        }
    }
}

fn plain_marker(label: &str) -> Marker {
    Marker {
        label: label.to_owned(),
        guardrail: None,
    }
}

pub(crate) fn hook(label: &str) -> Arc<dyn DeploymentHook> {
    Arc::new(plain_marker(label))
}

pub(crate) fn as_hook<T>(hook: Arc<T>) -> Arc<dyn DeploymentHook>
where
    T: DeploymentHook + 'static,
{
    hook
}

pub(crate) fn guardrail_hook(
    label: &str,
    name: Option<&str>,
    default_on: bool,
) -> Arc<dyn DeploymentHook> {
    Arc::new(Marker {
        label: label.to_owned(),
        guardrail: Some(Guardrail {
            name: name.map(str::to_owned),
            default_on,
        }),
    })
}

pub(crate) struct Rules<'rules> {
    pub(crate) rules: &'rules [Option<RuleDecision>],
}

impl RuleEvaluator for Rules<'_> {
    fn evaluate(&self, index: usize, _input: &str) -> Option<RuleDecision> {
        let rule = self.rules.get(index)?;
        Some(rule.clone().unwrap_or(RuleDecision::NotCallable))
    }
}

pub(crate) struct Credentials {
    pub(crate) entries: Vec<Credential>,
}

impl CredentialLookup for Credentials {
    fn find_credential(&self, credential_name: &str) -> Option<&Credential> {
        self.entries
            .iter()
            .find(|credential| credential.credential_name == credential_name)
    }
}

#[derive(Default)]
pub(crate) struct RecordingBreadcrumbs {
    pub(crate) entries: Mutex<Vec<(String, String, String)>>,
}

impl BreadcrumbSink for RecordingBreadcrumbs {
    fn add_breadcrumb(&self, category: &str, message: &str, level: &str) {
        self.entries
            .lock()
            .expect("the breadcrumb lock is free")
            .push((category.to_owned(), message.to_owned(), level.to_owned()));
    }
}

pub(crate) struct Setup {
    pub(crate) lists: CallbackLists,
    pub(crate) providers: ProviderRegistry,
    pub(crate) callbacks: Vec<Arc<dyn DeploymentHook>>,
    pub(crate) resolver: Arc<Resolver>,
    pub(crate) correlation: CorrelationContext,
    pub(crate) correlation_settings: CorrelationSettings,
    pub(crate) redactor: Redactor,
    pub(crate) breadcrumbs: Option<Arc<RecordingBreadcrumbs>>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) trace_id: String,
    pub(crate) is_async_call: bool,
    pub(crate) turn_off_message_logging: bool,
}

impl Setup {
    pub(crate) fn new(resolver: Resolver) -> Self {
        Self {
            lists: CallbackLists::default(),
            providers: ProviderRegistry::default(),
            callbacks: Vec::new(),
            resolver: Arc::new(resolver),
            correlation: CorrelationContext::default(),
            correlation_settings: CorrelationSettings {
                supports_correlation_logging: true,
                request_correlation_in_logs: false,
                secret_redaction: SecretRedaction::Enabled,
            },
            redactor: redactor(),
            breadcrumbs: None,
            litellm_params: IndexMap::new(),
            trace_id: String::from("generated-trace"),
            is_async_call: true,
            turn_off_message_logging: false,
        }
    }

    pub(crate) fn call(
        &mut self,
        original_function: &str,
        rules: &[Option<RuleDecision>],
        request: RequestMap,
    ) -> Result<CallSetup, CallSetupError> {
        let evaluator = Rules { rules };
        let resolver = Arc::clone(&self.resolver);
        let known = |name: &str| resolver.known(name);
        let loggers = CustomLoggers {
            known: &known,
            resolver: resolver.as_ref(),
        };
        let mut ticks = 0_u64;
        let mut clock = || {
            ticks = ticks.saturating_add(1);
            Duration::from_millis(ticks)
        };
        let trace_id = self.trace_id.clone();
        let next_trace_id = || trace_id.clone();
        let breadcrumbs: Option<&dyn BreadcrumbSink> = self
            .breadcrumbs
            .as_ref()
            .map(|sink| -> &dyn BreadcrumbSink { sink.as_ref() });
        let deps = CallSetupDependencies {
            lists: &mut self.lists,
            providers: &mut self.providers,
            callbacks: &self.callbacks,
            loggers,
            correlation: &mut self.correlation,
            correlation_settings: self.correlation_settings,
            redactor: &self.redactor,
            trace: TraceSource {
                next_trace_id: &next_trace_id,
                clock: &mut clock,
            },
            breadcrumbs,
            litellm_params: self.litellm_params.clone(),
            turn_off_message_logging: self.turn_off_message_logging,
            is_async_call: self.is_async_call,
        };
        function_setup(original_function, &evaluator, moment(), request, deps)
    }
}

pub(crate) fn setup() -> Setup {
    Setup::new(Resolver::default())
}

pub(crate) fn setup_with(outcomes: &[(&str, Outcome)]) -> Setup {
    Setup::new(Resolver::with(outcomes))
}

pub(crate) fn setup_message_texts(call_type: &str, extra: &str) -> Vec<String> {
    let mut fixture = setup();
    let result = fixture
        .call(call_type, &[], chat_request(extra))
        .expect("the setup succeeds");
    message_texts(result.logging.messages)
}

pub(crate) fn message_texts(messages: Option<Vec<Json>>) -> Vec<String> {
    let mut texts = Vec::new();
    for message in messages.unwrap_or_default() {
        if let Json::Object(fields) = message
            && let Some(Json::Text(text)) = fields.get("content")
        {
            texts.push(text.clone());
        }
    }
    texts
}

pub(crate) fn labels(list: &[DynamicCallback]) -> Vec<String> {
    list.iter()
        .map(|entry| match entry {
            DynamicCallback::Name(name) => format!("name:{name}"),
            DynamicCallback::Logger(logger) => format!("logger:{}", logger.callback_name()),
        })
        .collect()
}
