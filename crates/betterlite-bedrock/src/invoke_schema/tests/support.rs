use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::model_capability_predicates::CapabilityRequest;
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::invoke_schema::InvokeSchemaCapabilities;

// One recorded capability question: the model, the provider argument and the key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) model: String,
    pub(crate) provider: Option<String>,
    pub(crate) key: String,
}

#[derive(Default)]
pub(crate) struct Capabilities {
    pub(crate) factory_flags: Vec<&'static str>,
    pub(crate) effort: bool,
    pub(crate) queries: Vec<Query>,
    pub(crate) effort_models: Vec<String>,
}

impl Capabilities {
    pub(crate) fn with_flag(flag: &'static str) -> Self {
        let mut capabilities = Self::default();
        capabilities.factory_flags.push(flag);
        capabilities
    }
}

impl InvokeSchemaCapabilities for Capabilities {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.queries.push(Query {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.map(str::to_owned),
            key: key.to_owned(),
        });
        ready(self.factory_flags.contains(&key))
    }

    fn supports_effort_param(&mut self, model: &str) -> impl Future<Output = bool> + Send {
        self.effort_models.push(model.to_owned());
        ready(self.effort)
    }
}

#[derive(Clone, Default)]
pub(crate) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
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

pub(crate) fn capture_warnings() -> (DefaultGuard, Warnings) {
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    (set_default(subscriber), warnings)
}

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime")
}
