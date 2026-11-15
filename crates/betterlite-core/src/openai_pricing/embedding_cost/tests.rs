#![cfg(test)]

use std::fmt::Debug;
use std::io::Error as IoError;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::embedding::EmbeddingResponse;
use rust_decimal::Decimal;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::embedding_cost;
use crate::openai_pricing::{CompletionCost, CompletionCostRequest, CostError, CostedResponse};
use crate::routes::CallType;

const BODY: &str = r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":8,"total_tokens":8}}"#;

#[derive(Debug, PartialEq)]
struct Observed {
    model: String,
    provider: String,
    call_type: CallType,
    embedding: bool,
}

struct RecordingCost {
    charge: Decimal,
    fails: bool,
    observed: Mutex<Vec<Observed>>,
}

impl RecordingCost {
    fn ok(charge: Decimal) -> Self {
        Self {
            charge,
            fails: false,
            observed: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            charge: Decimal::ZERO,
            fails: true,
            observed: Mutex::new(Vec::new()),
        }
    }

    fn observed(&self) -> Vec<Observed> {
        mem::take(&mut *self.observed.lock().unwrap())
    }
}

impl CompletionCost for RecordingCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Result<Decimal, CostError> {
        self.observed.lock().unwrap().push(Observed {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.to_owned(),
            call_type: request.call_type,
            embedding: matches!(request.response, CostedResponse::Embedding(_)),
        });
        if self.fails {
            return Err(Box::new(IoError::other("model not mapped")));
        }
        Ok(self.charge)
    }
}

fn response() -> EmbeddingResponse {
    serde_json::from_str(BODY).unwrap()
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap()
}

#[test]
fn the_cost_service_sees_the_embedding_with_the_aembedding_call_type() {
    let cost = RecordingCost::ok(decimal("0.125"));
    let charge = embedding_cost(&response(), "text-embedding-3-small", "openai", &cost);
    assert_eq!(charge, decimal("0.125"));
    assert_eq!(
        cost.observed(),
        vec![Observed {
            model: String::from("text-embedding-3-small"),
            provider: String::from("openai"),
            call_type: CallType::Aembedding,
            embedding: true,
        }]
    );
}

#[test]
fn the_model_and_provider_are_passed_as_given() {
    let cost = RecordingCost::ok(Decimal::ZERO);
    let charge = embedding_cost(&response(), "azure/text-embedding-3-small", "azure", &cost);
    assert_eq!(charge, Decimal::ZERO);
    assert_eq!(
        cost.observed(),
        vec![Observed {
            model: String::from("azure/text-embedding-3-small"),
            provider: String::from("azure"),
            call_type: CallType::Aembedding,
            embedding: true,
        }]
    );
}

#[test]
fn a_failed_cost_returns_zero_and_warns() {
    let cost = RecordingCost::failing();
    let embedding = response();
    let (charge, logs) =
        capture(|| embedding_cost(&embedding, "unmapped-embedding-model", "openai", &cost));
    assert_eq!(charge, Decimal::ZERO);
    assert!(logs.iter().any(|line| line
        .contains("Error calculating embeddings cost for model unmapped-embedding-model, logging spend with cost 0")));
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
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

fn capture<T, F: FnOnce() -> T>(call: F) -> (T, Vec<String>) {
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}
