use super::CollectorClock;
use super::chunks::CoalescedChunks;
use super::event::decode_passthrough_log_event;
use super::record::{DecodedChunk, EventChunkDecoder, StreamLogError, generic_chunk};
use crate::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};
use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;
use crate::invoke_events::{
    DecoderClock, DecoderOptions, InvokeStreamDecoder, bedrock_event_stream_decoder,
};
use betterlite_models::decisions::Json;
use betterlite_openai::passthrough::transformation::types::{
    MAX_COLLECTED_BYTES, PassthroughError,
};
use betterlite_openai::passthrough::transformation::{
    PassthroughStreamCollector, types::PassthroughLogging,
};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use bytes::BytesMut;
pub trait StreamLogAssembler: Sync {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderFlags {
    pub sync_stream: bool,
    pub json_mode: bool,
}
pub const DECODER_FLAGS: DecoderFlags = DecoderFlags {
    sync_stream: true,
    json_mode: false,
};
pub enum DecoderChoice {
    Disabled {
        warning: Option<String>,
    },
    Converse(InvokeStreamDecoder),
    Invoke {
        provider: BedrockInvokeProvider,
        decoder: InvokeStreamDecoder,
    },
}
#[must_use]
pub fn decoder_selection(model: &str, endpoint: &str, clock: CollectorClock) -> DecoderChoice {
    let options = DecoderOptions {
        model,
        json_mode: Some(DECODER_FLAGS.json_mode),
        sync_stream: DECODER_FLAGS.sync_stream,
        clock: DecoderClock {
            next_id: clock.next_id,
            now: clock.now,
        },
    };
    if endpoint.contains("invoke") {
        let Some(provider) = invoke_provider(model) else {
            return DecoderChoice::Disabled {
                warning: Some(format!(
                    "Bedrock passthrough spend tracking skipped: no invoke provider for model {model}"
                )),
            };
        };
        return DecoderChoice::Invoke {
            provider,
            decoder: bedrock_event_stream_decoder(Some(provider.name()), &options),
        };
    }
    if endpoint.contains("converse") {
        return DecoderChoice::Converse(bedrock_event_stream_decoder(None, &options));
    }
    DecoderChoice::Disabled { warning: None }
}
#[must_use]
pub fn decoder_box(choice: DecoderChoice) -> Option<Box<dyn EventChunkDecoder>> {
    match choice {
        DecoderChoice::Disabled { warning } => {
            if let Some(message) = warning {
                tracing::warn!("{message}");
            }
            None
        }
        DecoderChoice::Converse(decoder) => Some(Box::new(decoder)),
        DecoderChoice::Invoke { decoder, .. } => Some(Box::new(decoder)),
    }
}
pub struct BedrockEventStreamCollector<'plan> {
    decoder: Option<Box<dyn EventChunkDecoder + 'plan>>,
    assembler: &'plan dyn StreamLogAssembler,
    clock: CollectorClock,
    shape: ResponseStreamShape,
    buffer: BytesMut,
    fed_bytes: usize,
    chunks: CoalescedChunks,
    failure: Option<StreamLogError>,
}
impl<'plan> BedrockEventStreamCollector<'plan> {
    #[must_use]
    pub fn new(
        decoder: Option<Box<dyn EventChunkDecoder + 'plan>>,
        assembler: &'plan dyn StreamLogAssembler,
        clock: CollectorClock,
    ) -> Self {
        Self {
            decoder,
            assembler,
            clock,
            shape: ResponseStreamShape::bedrock_runtime(),
            buffer: BytesMut::new(),
            fed_bytes: 0,
            chunks: CoalescedChunks::default(),
            failure: None,
        }
    }
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), StreamLogError> {
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(());
        };
        let Some(total) = self
            .fed_bytes
            .checked_add(chunk.len())
            .filter(|total| *total <= MAX_COLLECTED_BYTES)
        else {
            self.decoder = None;
            self.chunks = CoalescedChunks::default();
            self.buffer.clear();
            return Err(StreamLogError::TooLarge);
        };
        self.fed_bytes = total;
        self.buffer.extend_from_slice(chunk);
        loop {
            let Some(event) = decode_aws_event_frame(&mut self.buffer)? else {
                return Ok(());
            };
            let Some(message) = parse_message_from_event(Some(&self.shape), &event)? else {
                continue;
            };
            let value = decode_passthrough_log_event(message.as_bytes())?;
            let translated = match decoder.decode(&value, self.clock)? {
                DecodedChunk::Chunk(chunk) => Some(*chunk),
                DecodedChunk::Generic(value) => generic_chunk(&value, self.clock)?,
                DecodedChunk::Ignored => None,
            };
            if let Some(chunk) = translated {
                self.chunks.add(chunk);
            }
        }
    }
}
impl PassthroughStreamCollector for BedrockEventStreamCollector<'_> {
    type Logged = Json;
    fn add(&mut self, chunk: &[u8]) {
        if let Err(error) = self.feed(chunk) {
            self.failure = Some(error);
        }
    }
    fn build_logged_response(
        &self,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Result<Option<Self::Logged>, PassthroughError> {
        if self.failure.is_some() {
            return Ok(None);
        }
        let chunks = self.chunks.snapshot();
        if chunks.is_empty() {
            return Ok(None);
        }
        Ok(self.assembler.assemble(&chunks, logging_obj))
    }
}
