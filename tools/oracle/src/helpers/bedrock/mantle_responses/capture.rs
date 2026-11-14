use std::fmt;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::reasoning_models::ReasoningCapabilities;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

pub(super) struct Fixture;

impl ReasoningCapabilities for Fixture {
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

impl ResponsesCapabilities for Fixture {
    fn model_info(&mut self, _model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(ModelInfo::Unavailable)
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    pub(super) fn take(&self) -> Vec<String> {
        let mut messages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *messages)
    }
}

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
