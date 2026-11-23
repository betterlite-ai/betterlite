use betterlite_error::facts::bounded_text;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::responses_poll_state::{
    access::PollAccess,
    error::PollingError,
    get_cache_key,
    record::PollPatch,
    store::{ResponsePollingStore, StoredPollState, decode_responses_poll_state},
};

use super::ERROR_CODE;
use super::ERROR_TYPE;
use super::ROUTE_TYPE;
use super::accumulator::{AccumulatorLimitError, OutputAccumulator, check_value};
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
    Accumulated(#[from] AccumulatorLimitError),
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
        // the source's failed record: type, message and code, and no param
        let failure = Json::Object(IndexMap::from([
            (String::from("type"), Json::Text(String::from(ERROR_TYPE))),
            (String::from("message"), Json::Text(message)),
            (String::from("code"), Json::Text(String::from(ERROR_CODE))),
        ]));
        let mut fields = IndexMap::with_capacity(2);
        fields.insert(
            String::from("status"),
            Json::Text(String::from(FAILED_STATUS)),
        );
        fields.insert(String::from("error"), failure);
        self.write_patch(&PollPatch(fields)).await
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
            state.check_limits(self.store.limits)?;
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
            match decode_poll_stream_frame(&bytes, self.store.limits)? {
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
        if let Some(output) = state.flush_if_due(self.clock) {
            state.check_limits(self.store.limits)?;
            let patch = PollPatch(IndexMap::from([(String::from("output"), output)]));
            self.write_patch(&patch).await?;
        }
        Ok(())
    }

    async fn finish(&self, state: &OutputAccumulator) -> Result<(), PollStreamError> {
        state.check_limits(self.store.limits)?;
        self.write_patch(&state.final_patch()).await?;
        tracing::info!(
            "Finished background streaming for {}, status={}",
            self.polling_id,
            state.final_status()
        );
        Ok(())
    }

    // update_state applies the patch to the stored record, so the bound is measured on that whole record
    async fn write_patch(&self, patch: &PollPatch) -> Result<(), PollStreamError> {
        let Some(cache) = self.store.cache.as_ref() else {
            return Ok(());
        };
        let cached = cache
            .read(&get_cache_key(self.polling_id))
            .await
            .map_err(PollingError::from)?;
        let Some(bytes) = cached.filter(|bytes| !bytes.is_empty()) else {
            return Ok(());
        };
        let mut state =
            decode_responses_poll_state(&bytes, self.store.limits).map_err(PollingError::from)?;
        match &mut state {
            StoredPollState::Record(record) => patch.apply(&mut record.response),
            StoredPollState::Legacy(response) => patch.apply(response),
        }
        let encoded = serde_json::to_vec(&state).map_err(AccumulatorLimitError::Encode)?;
        let record =
            serde_json::from_slice::<Json>(&encoded).map_err(AccumulatorLimitError::Encode)?;
        check_value(&record, self.store.limits)?;
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
