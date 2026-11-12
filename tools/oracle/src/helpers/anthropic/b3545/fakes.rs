use std::convert::Infallible;
use std::future::{Future, Ready, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::native_compaction::anthropic::ModelInfoLookup;
use betterlite_anthropic::native_compaction::dispatch::{ChatCall, CompactionRouter};
use betterlite_anthropic::native_compaction::protocol::Mapping;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("model {model} is not in the bundled catalog")]
pub(super) struct UnknownModel {
    model: String,
}

pub(super) struct CatalogLookup {
    pub(super) catalog: Arc<ModelCatalog>,
}

impl ModelInfoLookup for CatalogLookup {
    type Error = UnknownModel;

    fn supports_anthropic_compaction(
        &self,
        model: &str,
        _custom_llm_provider: &str,
    ) -> impl Future<Output = Result<Option<bool>, UnknownModel>> + Send {
        ready(
            self.catalog
                .lookup
                .get(model)
                .and_then(|name| self.catalog.entries.get(name.as_ref()))
                .map(|entry| {
                    entry
                        .fields
                        .capabilities
                        .reasoning
                        .supports_anthropic_compaction
                })
                .ok_or_else(|| UnknownModel {
                    model: model.to_owned(),
                }),
        )
    }
}

pub(super) struct RecordingRouter {
    reply_id: String,
    calls: Mutex<Vec<serde_json::Value>>,
}

impl RecordingRouter {
    pub(super) const fn new(reply_id: String) -> Self {
        Self {
            reply_id,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn recorded(&self) -> Vec<serde_json::Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: serde_json::Value) -> Ready<Result<Mapping, Infallible>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        ready(Ok(Mapping::from([(
            "id".to_owned(),
            Json::Text(self.reply_id.clone()),
        )])))
    }
}

impl CompactionRouter for RecordingRouter {
    type Error = Infallible;

    fn aanthropic_messages(
        &self,
        payload: &Mapping,
    ) -> impl Future<Output = Result<Mapping, Infallible>> + Send {
        self.record(json!({"route": "messages", "options": payload}))
    }

    fn acompletion(
        &self,
        call: &ChatCall<'_>,
    ) -> impl Future<Output = Result<Mapping, Infallible>> + Send {
        self.record(json!({
            "route": "chat",
            "model": call.model,
            "messages": call.messages,
            "stream": call.stream,
            "options": call.options,
        }))
    }
}
