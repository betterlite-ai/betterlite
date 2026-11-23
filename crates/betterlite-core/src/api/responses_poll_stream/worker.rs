use betterlite_error::facts::bounded_text;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::responses_poll_state::access::PollAccess;
use crate::api::responses_poll_state::error::PollingError;
use crate::api::responses_poll_state::record::PollPatch;
use crate::api::responses_poll_state::store::ResponsePollingStore;

use super::ERROR_CODE;
use super::ERROR_TYPE;
use super::ROUTE_TYPE;
use super::accumulator::OutputAccumulator;
use super::clock::PollClock;
use super::event::{PollEvent, PollEventError};
use super::frame::{PollFrame, PollFrameError, decode_poll_stream_frame};
use super::ports::{
    DetachedRequest, PollChunkSource, PollProcessCall, PollProcessed, PollProcessorError,
    PollRequest, PollRequestProcessor,
};

const STATUS_IN_PROGRESS: &str = "in_progress";
const FAILED_STATUS: &str = "failed";

// background_streaming_task: stream a provider response into the polling record after admission
pub struct BackgroundStreaming<'context> {
    pub polling_id: &'context str,
    pub data: IndexMap<String, Json>,
    pub request: PollRequest,
    pub store: &'context ResponsePollingStore,
    pub processor: &'context dyn PollRequestProcessor,
    pub clock: &'context dyn PollClock,
    pub redactor: &'context Redactor,
    pub limits: ApiLimits,
}

// the source stores str(e), so every variant's text is the message the record carries
#[derive(Debug, Error)]
pub enum PollStreamError {
    #[error(transparent)]
    Processor(#[from] PollProcessorError),
    #[error(transparent)]
    Frame(#[from] PollFrameError),
    #[error(transparent)]
    Event(#[from] PollEventError),
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error(transparent)]
    Polling(#[from] PollingError),
}

impl BackgroundStreaming<'_> {
    // the source catches every failure once, records it and returns; only that record can fail again
    pub async fn run(self) -> Result<(), PollStreamError> {
        let Err(error) = self.stream().await else {
            return Ok(());
        };
        tracing::error!(
            "Error in background streaming task for {}: {}",
            self.polling_id,
            error
        );
        let message = bounded_text(&self.redactor.redact_internal_details(&error.to_string()));
        self.write_patch(&failure_patch(&message)).await
    }

    async fn stream(&self) -> Result<(), PollStreamError> {
        tracing::info!("Starting background streaming for {}", self.polling_id);
        self.write_patch(&status_patch(STATUS_IN_PROGRESS)).await?;
        let mut data = self.data.clone();
        data.insert(String::from("stream"), Json::Bool(true));
        data.shift_remove("background");
        let call = PollProcessCall {
            route_type: ROUTE_TYPE,
            model: None,
            skip_pre_call_logic: true,
            data: &data,
            request: DetachedRequest {
                scope: &self.request.scope,
            },
        };
        let processed = self.processor.process(call).await?;
        let mut state = OutputAccumulator::started(self.clock);
        let PollProcessed::Streaming(mut chunks) = processed else {
            tracing::warn!(
                "background_streaming_task: response for {} has no body_iterator; this may indicate a misconfiguration or provider error",
                self.polling_id
            );
            return self.finish(&state).await;
        };
        self.consume(chunks.as_mut(), &mut state).await?;
        if let Some(output) = state.flush_forced(self.clock) {
            let patch = PollPatch(IndexMap::from([(String::from("output"), output)]));
            self.write_patch(&patch).await?;
        }
        self.finish(&state).await
    }

    async fn consume(
        &self,
        chunks: &mut dyn PollChunkSource,
        state: &mut OutputAccumulator,
    ) -> Result<(), PollStreamError> {
        while let Some(bytes) = chunks.next_chunk().await? {
            match decode_poll_stream_frame(&bytes, self.limits)? {
                PollFrame::NoData => {}
                PollFrame::Done => break,
                PollFrame::Malformed => {
                    tracing::warn!("Failed to parse streaming chunk: the payload is not JSON");
                }
                PollFrame::Event(value) => self.apply_event(&value, state).await?,
            }
        }
        Ok(())
    }

    async fn apply_event(
        &self,
        value: &Json,
        state: &mut OutputAccumulator,
    ) -> Result<(), PollStreamError> {
        let event = PollEvent::decode(value)?;
        if matches!(&event, PollEvent::InProgress) {
            self.write_patch(&status_patch(STATUS_IN_PROGRESS)).await?;
        }
        state.apply(event)?;
        state.check_limits(self.limits)?;
        if let Some(output) = state.flush_if_due(self.clock) {
            let patch = PollPatch(IndexMap::from([(String::from("output"), output)]));
            self.write_patch(&patch).await?;
        }
        Ok(())
    }

    async fn finish(&self, state: &OutputAccumulator) -> Result<(), PollStreamError> {
        self.write_patch(&state.final_patch()).await?;
        tracing::info!(
            "Finished background streaming for {}, status={}",
            self.polling_id,
            state.final_status()
        );
        Ok(())
    }

    async fn write_patch(&self, patch: &PollPatch) -> Result<(), PollStreamError> {
        let update = self
            .store
            .update_state(self.polling_id, PollAccess::AuthDisabled, patch)
            .await?;
        tracing::debug!("Polling state for {}: {:?}", self.polling_id, update);
        Ok(())
    }
}

fn status_patch(status: &str) -> PollPatch {
    let mut fields = IndexMap::with_capacity(1);
    fields.insert(String::from("status"), Json::Text(status.to_owned()));
    PollPatch(fields)
}

// the source's failed record: type, message and code, and no param
fn failure_patch(message: &str) -> PollPatch {
    let error = Json::Object(IndexMap::from([
        (String::from("type"), Json::Text(String::from(ERROR_TYPE))),
        (String::from("message"), Json::Text(message.to_owned())),
        (String::from("code"), Json::Text(String::from(ERROR_CODE))),
    ]));
    let fields = IndexMap::from([
        (
            String::from("status"),
            Json::Text(String::from(FAILED_STATUS)),
        ),
        (String::from("error"), error),
    ]);
    PollPatch(fields)
}
