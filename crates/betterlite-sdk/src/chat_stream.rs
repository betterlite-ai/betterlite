pub mod config;
pub mod error;
pub mod ports;

mod chunk;
mod duration;
mod empty;
mod failure;
mod finalize;
mod hooks;
mod json;
mod mcp;
mod next;
mod tests;

use betterlite_streaming::provider_chunk_decode::chunk::{DecodeState, RequestOptions};
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use betterlite_streaming::stream_chunk_shaping::tokens::check_is_function_call;

use config::ChatStreamConfig;
use error::ChatStreamError;
use ports::{ChatStreamPorts, ChunkSource};

// Whether the stream still reads, handed its chunks over, or was closed under it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Active,
    Completed,
    Closed,
}

pub struct ChatStream<'ports, S> {
    config: ChatStreamConfig,
    source: S,
    ports: ChatStreamPorts<'ports>,
    shaper: ChunkShaper,
    decode: DecodeState,
    logged_model: String,
    started_at: f64,
    cache_hit: bool,
    logging_started: bool,
    lifecycle: Lifecycle,
}

impl<'ports, S: ChunkSource> ChatStream<'ports, S> {
    #[must_use]
    pub fn new(config: ChatStreamConfig, source: S, ports: ChatStreamPorts<'ports>) -> Self {
        let started_at = ports.clock.now().timestamp;
        let options = RequestOptions {
            send_stream_usage: config.send_stream_usage,
            is_function_call: check_is_function_call(config.optional_params.as_ref()),
        };
        let logged_model = config.effective_model();
        let shaper = ChunkShaper::new(
            logged_model.clone(),
            config.provider.clone(),
            config.merge_reasoning_in_choices,
            config.base_hidden_params(),
        );
        let decode = DecodeState {
            model: config.model.clone(),
            kind: config.provider_kind,
            received_finish_reason: None,
            intermittent_finish_reason: None,
            sent_first_chunk: false,
            complete_response: String::new(),
            completion_stream: config.completion_stream.clone(),
            options,
            repeated_chunk_limit: config.repeated_chunk_limit,
            next_id: shaper.next_id,
        };
        let cache_hit = config.provider.as_deref() == Some("cached_response");
        Self {
            config,
            source,
            ports,
            shaper,
            decode,
            logged_model,
            started_at,
            cache_hit,
            logging_started: false,
            lifecycle: Lifecycle::Active,
        }
    }

    pub async fn next(&mut self) -> Result<Option<StreamChunk>, ChatStreamError> {
        if !matches!(self.lifecycle, Lifecycle::Active) {
            let ports = &mut self.ports;
            ports
                .restore
                .restore_correlation_context(&mut *ports.correlation);
            return Ok(None);
        }
        let outcome = next::advance(self).await;
        if !matches!(outcome, Ok(Some(_))) {
            let ports = &mut self.ports;
            ports
                .restore
                .restore_correlation_context(&mut *ports.correlation);
        }
        outcome
    }

    pub async fn close(&mut self) {
        if !matches!(self.lifecycle, Lifecycle::Closed) {
            self.lifecycle = Lifecycle::Closed;
            if let Err(failure) = self.source.close().await {
                tracing::debug!(
                    %failure,
                    "CustomStreamWrapper.aclose: error closing completion_stream"
                );
            }
        }
        let ports = &mut self.ports;
        ports
            .restore
            .restore_correlation_context(&mut *ports.correlation);
    }
}
