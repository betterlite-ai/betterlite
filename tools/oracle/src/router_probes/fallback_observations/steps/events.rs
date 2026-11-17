use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::fallback_observations::error_info::exception_message;
use betterlite_router::fallback_observations::observer::{
    FallbackEvent, FallbackObservationError, FallbackObserver, FallbackOutcome, log_fallback_event,
};

use super::super::wire::{EventCall, EventCase, EventFound, ObserverKind, Outcome};
use super::errors::context;

pub(super) async fn events(cases: &[EventCase]) -> Vec<EventFound> {
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        found.push(event_found(case).await);
    }
    found
}

// log_success_fallback_event/log_failure_fallback_event hand every registered CustomLogger the same three values
async fn event_found(case: &EventCase) -> EventFound {
    let recording = Arc::new(Mutex::new(Vec::new()));
    let observers = observers(case, &recording);
    // log_success_fallback_event and log_failure_fallback_event pass a plain exception through to every observer
    let exception = ErrorClass::Unclassified(Box::new(context(case.message.clone())));
    log_fallback_event(
        &observers,
        outcome_of(case.outcome),
        FallbackEvent {
            original_model_group: &case.original_model_group,
            request: &case.request,
            exception: &exception,
        },
    )
    .await;
    let recorded = if let Ok(guard) = recording.lock() {
        guard.clone()
    } else {
        Vec::new()
    };
    EventFound { calls: recorded }
}

const fn outcome_of(outcome: Outcome) -> FallbackOutcome {
    match outcome {
        Outcome::Success => FallbackOutcome::Success,
        Outcome::Failure => FallbackOutcome::Failure,
    }
}

fn observers(
    case: &EventCase,
    calls: &Arc<Mutex<Vec<EventCall>>>,
) -> Vec<Arc<dyn FallbackObserver>> {
    case.observers
        .iter()
        .enumerate()
        .map(|(index, kind)| observer_for(*kind, u32::try_from(index).unwrap_or(u32::MAX), calls))
        .collect()
}

fn observer_for(
    kind: ObserverKind,
    index: u32,
    calls: &Arc<Mutex<Vec<EventCall>>>,
) -> Arc<dyn FallbackObserver> {
    match kind {
        ObserverKind::Record => Arc::new(RecordingObserver {
            index,
            calls: Arc::clone(calls),
        }),
        ObserverKind::Fail => Arc::new(FailingObserver),
        ObserverKind::Silent => Arc::new(SilentObserver),
    }
}

struct RecordingObserver {
    index: u32,
    calls: Arc<Mutex<Vec<EventCall>>>,
}

impl RecordingObserver {
    fn record(&self, outcome: Outcome, event: FallbackEvent<'_>) {
        let call = EventCall {
            observer: self.index,
            outcome,
            original_model_group: event.original_model_group.to_owned(),
            model: model_of(event.request),
            message: exception_message(event.exception),
        };
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }
}

fn model_of(request: &JsonObject) -> Option<String> {
    match request.get("model") {
        Some(Json::Text(model)) => Some(model.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

#[async_trait]
impl FallbackObserver for RecordingObserver {
    async fn log_success_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        self.record(Outcome::Success, event);
        Ok(())
    }

    async fn log_failure_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        self.record(Outcome::Failure, event);
        Ok(())
    }
}

struct FailingObserver;

fn observer_failure(event: &FallbackEvent<'_>) -> FallbackObservationError {
    FallbackObservationError::Observer {
        message: format!(
            "the observer failed for {} over {} values: {}",
            event.original_model_group,
            event.request.len(),
            exception_message(event.exception)
        ),
    }
}

#[async_trait]
impl FallbackObserver for FailingObserver {
    async fn log_success_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Err(observer_failure(&event))
    }

    async fn log_failure_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Err(observer_failure(&event))
    }
}

struct SilentObserver;

impl FallbackObserver for SilentObserver {}
