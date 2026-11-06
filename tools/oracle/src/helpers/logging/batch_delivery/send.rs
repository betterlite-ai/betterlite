use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Mutex, PoisonError};

use betterlite_logging::batch::delivery::{
    BatchTransport, DeliverySettings, SendFailure, send_batch_with_413_split,
};
use betterlite_logging::batch::retry::RequeuePolicy;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SendScenario {
    events: Vec<String>,
    success_status_codes: Vec<u16>,
    #[serde(default)]
    policy: Policy,
    #[serde(default)]
    max_events_per_request: Option<usize>,
    #[serde(default)]
    oversized: Vec<String>,
    #[serde(default)]
    unserializable: Vec<String>,
    script: Vec<ScriptStep>,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Policy {
    #[default]
    Undelivered,
    Requeue,
}

impl Policy {
    const fn requeue(self) -> RequeuePolicy {
        match self {
            Self::Undelivered => RequeuePolicy::Undelivered,
            Self::Requeue => RequeuePolicy::RequeueAll,
        }
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum ScriptStep {
    Response { status: u16 },
    StatusError { status: u16 },
    Error,
    Cancel,
}

impl ScriptStep {
    const fn outcome(self) -> &'static str {
        match self {
            Self::Response { .. } => "response",
            Self::StatusError { .. } => "status_error",
            Self::Error => "error",
            Self::Cancel => "cancel",
        }
    }

    const fn status(self) -> Option<u16> {
        match self {
            Self::Response { status } | Self::StatusError { status } => Some(status),
            Self::Error | Self::Cancel => None,
        }
    }

    const fn answer(self) -> Result<u16, SendFailure<ScriptedFailure>> {
        match self {
            Self::Response { status } => Ok(status),
            Self::StatusError { status } => Err(SendFailure::Status { status }),
            Self::Error => Err(SendFailure::Other(ScriptedFailure)),
            Self::Cancel => Err(SendFailure::Cancelled),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted transport failed")]
struct ScriptedFailure;

struct Attempt {
    events: Vec<String>,
    outcome: &'static str,
    status: Option<u16>,
}

struct ScriptedTransport {
    script: Mutex<VecDeque<ScriptStep>>,
    attempts: Mutex<Vec<Attempt>>,
    delivered: Mutex<Vec<String>>,
    success_status_codes: Vec<u16>,
    max_events_per_request: Option<usize>,
    oversized: Vec<String>,
    unserializable: Vec<String>,
}

impl BatchTransport<String> for ScriptedTransport {
    type SendError = ScriptedFailure;
    type LimitError = ScriptedFailure;

    fn send(
        &self,
        batch: &[String],
    ) -> impl Future<Output = Result<u16, SendFailure<Self::SendError>>> + Send {
        let step = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ScriptStep::Error);
        let events = batch.to_vec();
        if matches!(step, ScriptStep::Response { status } if self.success_status_codes.contains(&status))
        {
            self.delivered
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(events.iter().cloned());
        }
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Attempt {
                events,
                outcome: step.outcome(),
                status: step.status(),
            });
        async move { step.answer() }
    }

    fn exceeds_limits(&self, batch: &[String]) -> Result<bool, Self::LimitError> {
        if batch
            .iter()
            .any(|event| self.unserializable.contains(event))
        {
            return Err(ScriptedFailure);
        }
        if batch.iter().any(|event| self.oversized.contains(event)) {
            return Ok(true);
        }
        Ok(self
            .max_events_per_request
            .is_some_and(|max| batch.len() > max))
    }
}

fn attempt_values(transport: &ScriptedTransport) -> Vec<Value> {
    transport
        .attempts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|attempt| {
            json!({
                "events": attempt.events,
                "outcome": attempt.outcome,
                "status": attempt.status,
            })
        })
        .collect()
}

pub(super) async fn send_outcome(scenario: &SendScenario) -> Value {
    let transport = ScriptedTransport {
        script: Mutex::new(scenario.script.iter().copied().collect()),
        attempts: Mutex::new(Vec::new()),
        delivered: Mutex::new(Vec::new()),
        success_status_codes: scenario.success_status_codes.clone(),
        max_events_per_request: scenario.max_events_per_request,
        oversized: scenario.oversized.clone(),
        unserializable: scenario.unserializable.clone(),
    };
    let settings = DeliverySettings {
        success_status_codes: &scenario.success_status_codes,
        policy: scenario.policy.requeue(),
        integration_name: "BatchDelivery",
        detail: "scripted transport",
        drop_error_message: "batch delivery dropped a single event the destination rejected as too large",
    };
    let result = send_batch_with_413_split(&transport, &scenario.events, settings).await;
    let (undelivered, cancelled) = match result {
        Ok(undelivered) => (undelivered, false),
        Err(error) => (error.undelivered, true),
    };
    let delivered = transport
        .delivered
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({
        "attempts": attempt_values(&transport),
        "delivered": delivered,
        "undelivered": undelivered,
        "cancelled": cancelled,
    })
}
