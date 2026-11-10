use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_vertex::model_policy::capabilities::response_schema::{
    get_supports_response_schema, supports_response_json_schema,
};
use betterlite_vertex::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
    get_supports_system_message,
};
use serde_json::{Value, json};
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context as LayerContext;
use tracing_subscriber::layer::SubscriberExt as _;

use super::super::input::{CapabilityCase, LookupOutcome, ProviderName};

pub(super) fn capabilities(cases: &[CapabilityCase]) -> Value {
    let entries: Vec<Value> = cases.iter().map(capability_value).collect();
    json!({
        "cases": entries,
    })
}

fn capability_value(case: &CapabilityCase) -> Value {
    let lookup = FixtureLookup {
        system: case.system,
        schema: case.schema,
        provider: expected_provider(case.provider),
    };
    let counter = WarningCounter::default();
    let subscriber = tracing_subscriber::registry().with(counter.clone());
    let provider = into_provider(case.provider);
    let system = with_default(subscriber, || {
        get_supports_system_message(&case.model, provider, &lookup)
    });
    let schema = get_supports_response_schema(&case.model, provider, &lookup).map_or_else(
        |_error| json!({"error": true}),
        |supported| json!(supported),
    );
    json!({
        "system": system,
        "system_warned": counter.0.load(Ordering::Relaxed) > 0,
        "schema": schema,
        "json_schema": supports_response_json_schema(&case.model),
    })
}

const fn into_provider(name: ProviderName) -> VertexCapabilityProvider {
    match name {
        ProviderName::VertexAi => VertexCapabilityProvider::VertexAi,
        ProviderName::VertexAiBeta => VertexCapabilityProvider::VertexAiBeta,
        ProviderName::Gemini => VertexCapabilityProvider::Gemini,
    }
}

const fn expected_provider(name: ProviderName) -> &'static str {
    match name {
        ProviderName::VertexAi | ProviderName::VertexAiBeta => "vertex_ai",
        ProviderName::Gemini => "gemini",
    }
}

struct FixtureLookup {
    system: LookupOutcome,
    schema: LookupOutcome,
    provider: &'static str,
}

impl ModelCapabilityLookup for FixtureLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        self.check_provider(provider)?;
        let outcome = match kind {
            CapabilityKind::SystemMessages => self.system,
            CapabilityKind::ResponseSchema => self.schema,
        };
        outcome.value()
    }
}

impl FixtureLookup {
    fn check_provider(&self, provider: &str) -> Result<(), CapabilityLookupError> {
        if provider == self.provider {
            return Ok(());
        }
        Err(CapabilityLookupError {
            message: format!("expected provider {}", self.provider),
        })
    }
}

impl LookupOutcome {
    fn value(self) -> Result<bool, CapabilityLookupError> {
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::Error => Err(CapabilityLookupError {
                message: "capability lookup failed".to_owned(),
            }),
        }
    }
}

#[derive(Clone, Default)]
struct WarningCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for WarningCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _context: LayerContext<'_, S>) {
        if event.metadata().level() == &tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}
