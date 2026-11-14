use std::sync::Arc;

use betterlite_core::api::messages_native_stream::registry::StreamRegistry;
use indexmap::IndexMap;

use super::super::Http;
use super::super::recording::Warnings;
use super::Session;
use super::call::{MessagesNativeStreamCall, Observation, RelayOp, RelayStep, StepOutcome};
use crate::sdk_http::error::SdkHttpError;

pub(super) struct Run<'run> {
    pub(super) call: &'run MessagesNativeStreamCall,
    pub(super) http: &'run Http<'run>,
    pub(super) registry: Arc<StreamRegistry>,
    pub(super) warnings: Warnings,
    pub(super) sessions: IndexMap<String, Session>,
}

impl Run<'_> {
    pub(super) async fn apply(&mut self, step: &RelayStep) -> Result<StepOutcome, SdkHttpError> {
        match step.op {
            RelayOp::Settle => Ok(StepOutcome {
                frame: None,
                observation: Some(self.settle().await?),
            }),
            RelayOp::Start => {
                let session =
                    Session::start(self.call, self.http, Arc::clone(&self.registry)).await?;
                self.sessions.insert(step.stream.clone(), session);
                Ok(StepOutcome::default())
            }
            RelayOp::Release
            | RelayOp::Read
            | RelayOp::Detach
            | RelayOp::Finish
            | RelayOp::Fail
            | RelayOp::Arm
            | RelayOp::Wait => {
                let session = self.sessions.get_mut(&step.stream).ok_or_else(|| {
                    SdkHttpError::Invalid("the step names an unknown stream".to_owned())
                })?;
                Ok(StepOutcome {
                    frame: session.apply(step).await?,
                    observation: None,
                })
            }
        }
    }

    // Billing for every stream is parked or queued by its pump; the worker is drained once all ran
    async fn settle(&self) -> Result<Observation, SdkHttpError> {
        for session in self.sessions.values() {
            let mut viewer = session.state.recorded.subscribe();
            drop(viewer.wait_for(|value| *value).await);
            session.worker.flush().await;
        }
        let streams = self
            .sessions
            .iter()
            .map(|(name, session)| session.observe().map(|seen| (name.clone(), seen)))
            .collect::<Result<IndexMap<_, _>, _>>()?;
        Ok(Observation {
            warnings: self.warnings.take()?,
            streams,
        })
    }
}
