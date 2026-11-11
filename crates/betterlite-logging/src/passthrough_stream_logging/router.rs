use std::sync::{Arc, PoisonError};

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use tokio::task::{JoinError, JoinSet};

// ast-grep-ignore: no-long-names
use super::build::{StreamLoggingInput, StreamLoggingOutput, build_passthrough_logging_result};
use super::lines::convert_raw_bytes_to_str_lines;
use super::object::{StreamLoggingObject, StreamSuccessCall};
use super::sse_frames::{split_complete_sse_frames, tail_is_blank};
use super::tinyfish::{TinyfishCollectedChunks, TinyfishStreamBilling};
use super::types::{EndpointType, HandlerError, ProviderChunkHandlers};
use crate::logging_success::types::SuccessResult;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;

const PROMPT_CACHE_RESPONSE_COMPLETE: &str = "prompt_cache_response_complete";
const ROUTE_ERROR_PREFIX: &str = "Error in _route_streaming_logging_to_handler:";

pub trait AnthropicFrameProbe: Send + Sync {
    fn is_message_stop(&self, frames: &[u8]) -> bool;
    fn is_provider_error(&self, frames: &[u8]) -> bool;
}

pub struct StreamLoggingRouter {
    pub providers: Arc<ProviderChunkHandlers>,
    pub frames: Arc<dyn AnthropicFrameProbe>,
    pub tinyfish: Arc<dyn TinyfishStreamBilling>,
}

impl StreamLoggingRouter {
    // ast-grep-ignore: no-long-names
    pub async fn route_streaming_logging_to_handler(&self, input: StreamLoggingInput) {
        let joined = input.raw_bytes.concat();
        self.stamp_prompt_cache_complete(&input, &joined);
        if let Err(error) = self.route_guarded(input, joined).await {
            tracing::error!("{ROUTE_ERROR_PREFIX} {error}");
        }
    }

    fn stamp_prompt_cache_complete(&self, input: &StreamLoggingInput, joined: &[u8]) {
        let split: &[u8] = if input.endpoint_type == EndpointType::Anthropic {
            joined
        } else {
            &[]
        };
        let (frames, tail) = split_complete_sse_frames(split);
        let stopped = self.frames.is_message_stop(frames);
        let provider_error = self.frames.is_provider_error(frames);
        let complete = input.endpoint_type == EndpointType::Anthropic
            && tail_is_blank(tail)
            && stopped
            && !provider_error;
        input.logging_obj.edit(&mut |state| {
            state.model_call_details.insert(
                PROMPT_CACHE_RESPONSE_COMPLETE.to_owned(),
                Json::Bool(complete),
            );
        });
    }

    async fn route_guarded(
        &self,
        input: StreamLoggingInput,
        joined: Vec<u8>,
    ) -> Result<(), HandlerError> {
        if input.endpoint_type == EndpointType::Tinyfish {
            return self.route_tinyfish(input, &joined).await;
        }
        let logging_obj = Arc::clone(&input.logging_obj);
        let start_time = input.start_time;
        let end_time = input.end_time;
        let mut tasks = JoinSet::new();
        // ast-grep-ignore: no-long-names
        tasks.spawn_blocking(move || build_passthrough_logging_result(&input));
        let output = join_build(tasks.join_next().await)?;
        let cache_hit = cache_hit_from_state(logging_obj.as_ref());
        dispatch_output(
            logging_obj.as_ref(),
            StreamDispatch {
                result: SuccessResult::from(output.result),
                kwargs: output.kwargs,
                start_time,
                end_time,
                cache_hit,
            },
        )
        .await;
        Ok(())
    }

    // A running poller and a run_id found after the stream own the billing, so this path logs nothing.
    fn tinyfish_owned_elsewhere(&self, input: &StreamLoggingInput, joined: &[u8]) -> bool {
        if self.tinyfish.sse_poller_spawned(input.logging_obj.as_ref()) {
            return true;
        }
        let Some(run_id) = self.tinyfish.run_id_from_sse_frames(joined) else {
            return false;
        };
        self.tinyfish
            .start_sse_run_billing(&run_id, input.logging_obj.as_ref(), input.start_time);
        true
    }

    async fn route_tinyfish(
        &self,
        input: StreamLoggingInput,
        joined: &[u8],
    ) -> Result<(), HandlerError> {
        if self.tinyfish_owned_elsewhere(&input, joined) {
            return Ok(());
        }
        let lines = convert_raw_bytes_to_str_lines(&input.raw_bytes);
        let call = TinyfishCollectedChunks {
            logging_obj: input.logging_obj.as_ref(),
            url_route: &input.url_route,
            start_time: input.start_time,
            end_time: input.end_time,
            lines: &lines,
        };
        let outcome = self
            .tinyfish
            .handle_logging_tinyfish_collected_chunks(&call)
            .await?;
        let result = outcome
            .result
            .map_or_else(|| SuccessResult::Other(Json::Null), SuccessResult::from);
        let cache_hit = cache_hit_from_state(input.logging_obj.as_ref());
        dispatch_output(
            input.logging_obj.as_ref(),
            StreamDispatch {
                result,
                kwargs: outcome.kwargs,
                start_time: input.start_time,
                end_time: input.end_time,
                cache_hit,
            },
        )
        .await;
        Ok(())
    }
}

fn cache_hit_from_state(logging_obj: &StreamLoggingObject) -> bool {
    logging_obj
        .read(&mut |state| state.model_call_details.get("cache_hit") == Some(&Json::Bool(true)))
        .unwrap_or(false)
}

struct StreamDispatch {
    result: SuccessResult,
    kwargs: PassThroughLoggingKwargs,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: bool,
}

async fn dispatch_output(logging_obj: &StreamLoggingObject, dispatch: StreamDispatch) {
    let taken = logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let Some(mut state) = taken else {
        tracing::error!("the stream logging state was taken before the success dispatch");
        return;
    };
    logging_obj
        .dispatch
        .dispatch_success(StreamSuccessCall {
            state: &mut state.logging,
            correlation: &mut state.correlation,
            result: dispatch.result,
            start_time: dispatch.start_time,
            end_time: dispatch.end_time,
            cache_hit: dispatch.cache_hit,
            kwargs: &dispatch.kwargs,
        })
        .await;
    *logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(state);
}

fn join_build(
    joined: Option<Result<Result<StreamLoggingOutput, HandlerError>, JoinError>>,
) -> Result<StreamLoggingOutput, HandlerError> {
    match joined {
        Some(Ok(Ok(output))) => Ok(output),
        Some(Ok(Err(error))) => Err(error),
        Some(Err(source)) => Err(HandlerError::BlockingTask { source }),
        None => Err(HandlerError::BlockingTaskLost),
    }
}
