use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::compression_interception::engine::{
    CompressionCall, CompressionEngine, CompressionOutput,
};
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::session::Session;
use super::{Call, CompressCallRecord, DiagnosticRecord};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::kinds::SdkResult;

const SOURCE: &str = "betterlite_core::api::compression_interception";
pub(super) const CALL_ID_FIELD: &str = "litellm_call_id";
pub(super) const GENERATED_ID: &str = "<generated>";

#[derive(Debug, Error)]
pub(super) enum ScriptedEngineError {
    #[error("the case scripted no compression result for this call")]
    Unscripted,
}

#[derive(Default)]
pub(super) struct ScriptedEngine {
    pub(super) pending: Mutex<Option<CompressionOutput>>,
    pub(super) calls: Mutex<Vec<CompressCallRecord>>,
}

#[async_trait]
impl CompressionEngine for ScriptedEngine {
    type Failure = ScriptedEngineError;

    async fn compress(
        &self,
        call: &CompressionCall<'_>,
    ) -> Result<CompressionOutput, ScriptedEngineError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CompressCallRecord {
                model: call.model.to_owned(),
                call_type: call.call_type,
                compression_trigger: call.compression_trigger,
                compression_target: call.compression_target,
                embedding_model: call.embedding_model.map(str::to_owned),
                embedding_model_params: call.embedding_model_params.cloned(),
            });
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or(ScriptedEngineError::Unscripted)
    }
}

// Only events the hook names with a kind are diagnostics, as the Python side keeps only its messages.
#[derive(Clone, Default)]
pub(super) struct Diagnostics(pub(super) Arc<Mutex<Vec<DiagnosticRecord>>>);

#[derive(Default)]
struct Fields {
    kind: Option<String>,
    reason: Option<String>,
}

impl Fields {
    fn record(&mut self, name: &str, value: String) {
        match name {
            "kind" => self.kind = Some(value),
            "reason" => self.reason = Some(value),
            _ => {}
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field.name(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record(field.name(), format!("{value:?}"));
    }
}

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if !event.metadata().target().starts_with(SOURCE) {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let Some(kind) = fields.kind else {
            return;
        };
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
            .push(DiagnosticRecord {
                level: level.to_owned(),
                kind,
                reason: fields.reason,
            });
    }
}

pub(crate) async fn send(
    call: &Call,
    _api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let session = Session::new(env);
    let _subscription = set_default(registry().with(session.diagnostics.clone()));
    let outcome = session.run(call).await?;
    Ok(SdkResult::CompressionInterception {
        value: Box::new(outcome),
    })
}
