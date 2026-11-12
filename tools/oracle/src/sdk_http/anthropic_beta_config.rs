use std::collections::BTreeMap;
use std::fmt::Debug;
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::anthropic_beta_config::BetaConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::error::SdkHttpError;
use super::result::{BetaConfigRun, BetaStepOutcome, Diagnostic, SdkResult};
use super::vector_query_embedding::outbound_client;
use super::{AnthropicBetaConfigCall, BetaConfigStep, BetaConfigUrl};

const SOURCE: &str = "betterlite_core::api::anthropic_beta_config";

// only events that name a kind are diagnostics; the Python side keeps the LiteLLM messages that map to the same kinds
#[derive(Clone, Default)]
struct Diagnostics(Arc<Mutex<Vec<Diagnostic>>>);

#[derive(Default)]
struct KindVisitor(Option<String>);

impl Visit for KindVisitor {
    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "kind" {
            self.0 = Some(value.to_owned());
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = KindVisitor::default();
        event.record(&mut visitor);
        let Some(kind) = visitor.0 else {
            return;
        };
        if !event.metadata().target().starts_with(SOURCE) {
            return;
        }
        let level = match *event.metadata().level() {
            Level::ERROR => "error",
            Level::WARN => "warning",
            Level::INFO => "info",
            Level::DEBUG => "debug",
            Level::TRACE => "trace",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Diagnostic {
                level: level.to_owned(),
                kind,
            });
    }
}

#[derive(Serialize)]
struct Document<'document> {
    litellm_settings: Section<'document>,
}

#[derive(Serialize)]
struct Section<'section> {
    anthropic_beta_headers_url: &'section str,
}

// the endpoint is the harness mock or empty, and the settings come from the real config loader
pub(crate) async fn send(
    call: &AnthropicBetaConfigCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let url = match call.url {
        BetaConfigUrl::Mock => format!("{api_base}/anthropic_beta_headers_config.json"),
        BetaConfigUrl::Empty => String::new(),
    };
    let text = serde_json::to_string(&Document {
        litellm_settings: Section {
            anthropic_beta_headers_url: &url,
        },
    })
    .map_err(SdkHttpError::Serialize)?;
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load(&text)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let loader = BetaConfigLoader::new(
        outbound_client(env)?,
        config.sections.anthropic_beta_config.clone(),
        ApiLimits::default(),
    );
    let diagnostics = Diagnostics::default();
    let _subscription = set_default(registry().with(diagnostics.clone()));
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        let matrix = match step {
            BetaConfigStep::Load => loader.matrix().await,
            BetaConfigStep::Reload => loader.reload().await,
        };
        steps.push(BetaStepOutcome {
            matrix: serde_json::to_value(&*matrix).map_err(SdkHttpError::Serialize)?,
            diagnostics: take(&mut *diagnostics.0.lock().unwrap_or_else(PoisonError::into_inner)),
        });
    }
    Ok(SdkResult::BetaConfig {
        value: BetaConfigRun { steps },
    })
}
