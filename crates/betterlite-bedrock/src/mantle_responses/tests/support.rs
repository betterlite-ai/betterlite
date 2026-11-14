use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_context::local_ttl_cache::dual::{CacheEvent, Diagnostics, DualLocalCache};
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities};
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_openai::response_params::params::ResponsesOptions;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::aws_settings::credential_cache::CredentialCache;
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_responses::config::{CUSTOM_LLM_PROVIDER, MantleResponsesConfig};
use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::options::map_openai_params;

pub(super) struct Capabilities;

impl ReasoningCapabilities for Capabilities {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
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

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn catalogue_declares_default(&mut self) -> bool {
        false
    }
}

impl ResponsesCapabilities for Capabilities {
    fn model_info(&mut self, model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(ModelInfo::Found {
            key: model.to_owned(),
            supports_reasoning: Some(true),
        })
    }
}

pub(super) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("fixture object")
}

pub(super) fn items(source: &str) -> Vec<Json> {
    serde_json::from_str(source).expect("fixture items")
}

pub(super) fn globals() -> MantleGlobals {
    MantleGlobals {
        mantle_api_key: None,
        mantle_api_base: None,
        mantle_region: None,
        aws_bearer_token: None,
        aws_region_name: None,
        aws_region: None,
    }
}

pub(super) fn config(use_openai_path: bool) -> MantleResponsesConfig {
    MantleResponsesConfig {
        credential_cache: None,
        use_openai_path,
    }
}

pub(super) fn config_with_cache(cache: Arc<CredentialCache>) -> MantleResponsesConfig {
    MantleResponsesConfig {
        credential_cache: Some(cache),
        use_openai_path: true,
    }
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

pub(super) fn headers() -> IndexMap<String, String> {
    let mut headers = IndexMap::new();
    headers.insert(
        "Authorization".to_owned(),
        "Bearer caller-fixture-key".to_owned(),
    );
    headers
}

pub(super) fn header(headers: &IndexMap<String, String>, name: &str) -> Option<String> {
    headers.get(name).cloned()
}

pub(super) fn map_with(
    settings: &MantleResponsesConfig,
    model: &str,
    params: &str,
    drop_request: bool,
) -> Result<Object, MantleResponsesError> {
    let options = ResponsesOptions {
        provider: CUSTOM_LLM_PROVIDER,
        model: model.to_owned(),
        params: serde_json::from_str(params).expect("fixture params"),
    };
    let mut capabilities = Capabilities;
    let mut bundled = BundledReasoningModels::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let drop_params = DropParams {
        global: false,
        request: drop_request,
    };
    runtime.block_on(map_openai_params(
        settings,
        &mut capabilities,
        &options,
        drop_params,
        &mut bundled,
    ))
}

pub(super) fn map(
    model: &str,
    params: &str,
    drop_request: bool,
) -> Result<Object, MantleResponsesError> {
    map_with(&config(true), model, params, drop_request)
}

pub(super) fn mapped(model: &str, params: &str, drop_request: bool) -> Object {
    map(model, params, drop_request).expect("the params map")
}

pub(super) fn rejected(model: &str, params: &str, drop_request: bool) -> MantleResponsesError {
    match map(model, params, drop_request) {
        Ok(mapped) => panic!("expected a rejection, mapped {mapped:?}"),
        Err(error) => error,
    }
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
        let messages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut found = Vec::new();
        for (kind, text) in messages.iter() {
            if *kind == level {
                found.push(text.clone());
            }
        }
        found
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
