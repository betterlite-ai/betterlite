use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use secrecy::SecretString;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::responses::error::ResponsesError;
use crate::responses::options::map_openai_params;

pub(super) const MODEL: &str = "openai.gpt-oss-120b";

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
    fn model_info(&mut self, _model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(ModelInfo::Unavailable)
    }
}

// Answers from a URL map and records every call, so a test sees what the transform fetched and in which shape
pub(super) struct MapFetcher {
    pub(super) data_urls: BTreeMap<String, String>,
    pub(super) singles: Mutex<Vec<String>>,
    pub(super) batches: Mutex<Vec<Vec<String>>>,
}

impl MapFetcher {
    pub(super) fn with(pairs: &[(&str, &str)]) -> Self {
        Self {
            data_urls: pairs
                .iter()
                .map(|(url, data)| ((*url).to_owned(), (*data).to_owned()))
                .collect(),
            singles: Mutex::new(Vec::new()),
            batches: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Rejected {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for MapFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.singles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        ready(self.answer(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(urls.to_vec());
        ready(urls.iter().map(|url| self.answer(url)).collect())
    }
}

pub(super) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("fixture object")
}

pub(super) fn input(source: &str) -> ResponsesInput {
    serde_json::from_str(source).expect("fixture input")
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text)
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

pub(super) fn map(model: &str, params: &str, drop_request: bool) -> Result<Object, ResponsesError> {
    let parsed = serde_json::from_str(params).map_err(ResponsesError::Encode)?;
    let drop_params = DropParams {
        global: false,
        request: drop_request,
    };
    block_on(map_openai_params(
        &mut Capabilities,
        model,
        &parsed,
        drop_params,
        &mut BundledReasoningModels::default(),
    ))
}

pub(super) fn mapped(params: &str) -> Object {
    map(MODEL, params, false).expect("the params map")
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
    pub(super) fn warnings(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(level, _)| *level == Level::WARN)
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
