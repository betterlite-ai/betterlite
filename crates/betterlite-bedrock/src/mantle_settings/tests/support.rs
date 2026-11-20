use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_context::local_ttl_cache::dual::{CacheEvent, Diagnostics, DualLocalCache};
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers, Object};
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::aws_settings::credential_cache::CredentialCache;
use crate::aws_settings::sources::Credentials;
use crate::mantle_inputs::records::MantleGlobals;

pub(super) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("fixture object")
}

pub(super) fn globals(environment: &BTreeMap<String, String>) -> MantleGlobals {
    MantleGlobals {
        mantle_api_key: environment
            .get("BEDROCK_MANTLE_API_KEY")
            .cloned()
            .map(SecretString::from),
        mantle_api_base: environment.get("BEDROCK_MANTLE_API_BASE").cloned(),
        mantle_region: environment.get("BEDROCK_MANTLE_REGION").cloned(),
        aws_bearer_token: environment
            .get("AWS_BEARER_TOKEN_BEDROCK")
            .cloned()
            .map(SecretString::from),
        aws_region_name: environment.get("AWS_REGION_NAME").cloned(),
        aws_region: environment.get("AWS_REGION").cloned(),
    }
}

pub(super) fn empty_globals() -> MantleGlobals {
    MantleGlobals::default()
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

pub(super) fn catalog(chat_models: &[&str]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: chat_models.iter().map(|name| (*name).to_owned()).collect(),
        text_models: BTreeSet::new(),
    }
}

pub(super) fn headers(source: &[(&str, &str)]) -> Headers {
    source
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn header(source: &Headers, name: &str) -> Option<String> {
    source.get(name).cloned()
}

pub(super) fn credentials(access_key_id: &str, secret_access_key: &str) -> Credentials {
    Credentials {
        access_key_id: secret(access_key_id),
        secret_access_key: secret(secret_access_key),
        session_token: None,
    }
}

pub(super) fn fixture_credentials() -> Credentials {
    credentials("fixture-access-key", "fixture-secret-key")
}

pub(super) fn credential_cache() -> Arc<CredentialCache> {
    let memory = LocalTtlCache::with_clock(Some(1000), None, None, 1024, Arc::new(|| 0.0_f64))
        .expect("the fixture cache builds");
    let emit = Arc::new(|_event: CacheEvent| {});
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(16).expect("the fixture redactor builds")),
        emit,
    };
    Arc::new(
        CredentialCache::new(DualLocalCache {
            memory: Arc::new(Mutex::new(memory)),
            default_in_memory_ttl: None,
            diagnostics,
        })
        .expect("the credential cache takes capacity 1000"),
    )
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fixture runtime builds")
        .block_on(future)
}

#[derive(Clone, Copy)]
pub(super) enum Answer {
    Supported,
    Unsupported,
    Panic,
}

pub(super) type RecordedRequest = (String, Option<String>, String);

#[derive(Clone, Default)]
pub(super) struct Requests(Arc<Mutex<Vec<RecordedRequest>>>);

impl Requests {
    pub(super) fn seen(&self) -> Vec<RecordedRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct Policy {
    pub(super) answer: Answer,
    pub(super) requests: Requests,
}

impl Policy {
    pub(super) fn new(answer: Answer) -> Self {
        Self {
            answer,
            requests: Requests::default(),
        }
    }
}

impl CapabilityPredicates for Policy {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.requests
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                request.model.to_owned(),
                request.custom_llm_provider.map(str::to_owned),
                key.to_owned(),
            ));
        let answer = self.answer;
        async move {
            match answer {
                Answer::Supported => key == "supports_reasoning",
                Answer::Unsupported => false,
                Answer::Panic => panic!("fixture reasoning probe"),
            }
        }
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

#[derive(Clone, Default)]
pub(super) struct Messages(Arc<Mutex<Vec<(Level, String)>>>);

struct Recorded {
    message: String,
}

impl Visit for Recorded {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

impl Messages {
    pub(super) fn at(&self, level: Level) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(kind, _)| *kind == level)
            .map(|(_, text)| text.clone())
            .collect()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Messages {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut recorded = Recorded {
            message: String::new(),
        };
        event.record(&mut recorded);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), recorded.message));
    }
}

pub(super) fn capture() -> (Messages, DefaultGuard) {
    let messages = Messages::default();
    let guard = set_default(tracing_subscriber::registry().with(messages.clone()));
    (messages, guard)
}
