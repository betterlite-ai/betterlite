use std::future::Future;
use std::pin::Pin;

use betterlite_core::api::realtime_guardrails::callback::RealtimeGuardrail;
use betterlite_core::api::realtime_guardrails::runner::{
    RealtimeGuardrails, RunRealtimeGuardrails as _,
};
use serde_json::{Value, json};

use super::super::input::RunDocument;
use super::super::scripted::ScriptedError;
use super::{ScriptedConfig, Session, frame_log, label};

type RunFuture<'run, T> = Pin<Box<dyn Future<Output = T> + 'run>>;

impl Session {
    // One scripted runner call and the frames it produced; the runner borrows the state and is dropped before the frames are read
    pub(super) fn step<'run>(
        &'run mut self,
        callbacks: &'run [&'run dyn RealtimeGuardrail<Error = ScriptedError>],
        document: &'run RunDocument,
    ) -> RunFuture<'run, (Value, bool)> {
        Box::pin(async move {
            let outcome = {
                let mut runner: RealtimeGuardrails<'_, ScriptedConfig, ScriptedError> =
                    RealtimeGuardrails {
                        state: &mut self.state,
                        provider: None,
                        guardrails: callbacks,
                    };
                runner
                    .run_realtime_guardrails(
                        &document.transcript,
                        document.item_id.as_deref(),
                        document.pre_block_backend_message.as_deref(),
                        document.event_hooks.as_deref(),
                    )
                    .await
            };
            let (backend, client) = frame_log(self);
            match outcome {
                Ok(blocked) => {
                    let frame = outcome_frame(Some(blocked), &Value::Null, &backend, &client);
                    (frame, false)
                }
                Err(error) => {
                    let frame = outcome_frame(None, &Value::from(label(&error)), &backend, &client);
                    (frame, true)
                }
            }
        })
    }
}

fn outcome_frame(
    blocked: Option<bool>,
    error: &Value,
    backend: &[Value],
    client: &[Value],
) -> Value {
    json!({
        "blocked": blocked,
        "error": error,
        "backend": backend,
        "client": client,
    })
}
