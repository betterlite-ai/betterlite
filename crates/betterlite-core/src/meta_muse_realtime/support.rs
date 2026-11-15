#![cfg(test)]

use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::vec::IntoIter;

use futures_util::future::BoxFuture;
use tracing::Event;
use tracing::field::Field;
use tracing::field::Visit;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::Subscriber;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::Context;
use tracing_subscriber::layer::Layer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::request::Monotonic;
use super::request::Sleep;
use crate::api::transcription_protocol::events::EventIdSource;

pub(crate) struct FixedIds(IntoIter<String>);

impl From<&[&str]> for FixedIds {
    fn from(ids: &[&str]) -> Self {
        Self(
            ids.iter()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>()
                .into_iter(),
        )
    }
}

impl EventIdSource for FixedIds {
    fn next_event_id(&mut self) -> String {
        self.0.next().unwrap_or_default()
    }
}

#[derive(Default)]
pub(crate) struct CountingIds(usize);

impl EventIdSource for CountingIds {
    fn next_event_id(&mut self) -> String {
        self.0 = self.0.saturating_add(1);
        format!("event_{}", self.0)
    }
}

pub(crate) struct PacingFakes {
    pub(crate) clock: Monotonic,
    pub(crate) sleep: Sleep,
    pub(crate) delays: Arc<Mutex<Vec<f64>>>,
}

#[must_use]
pub(crate) fn pacing_fakes(ticks: &[f64]) -> PacingFakes {
    let readings: VecDeque<f64> = ticks.iter().copied().collect();
    let clock_readings = Arc::new(Mutex::new(readings));
    let delays: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
    let clock: Monotonic = Arc::new(move || {
        clock_readings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    });
    let recorded = Arc::clone(&delays);
    let sleep: Sleep = Arc::new(move |delay: f64| -> BoxFuture<'static, ()> {
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
        Box::pin(async {})
    });
    PacingFakes {
        clock,
        sleep,
        delays,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Capture(pub(crate) Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if !visitor.0.is_empty() {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(visitor.0);
        }
    }
}

pub(crate) fn capture() -> (Capture, DefaultGuard) {
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}

pub(crate) fn captured_messages(captured: &Capture) -> Vec<String> {
    captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
