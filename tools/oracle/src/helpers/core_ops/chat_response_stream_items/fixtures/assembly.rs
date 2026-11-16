use betterlite_core::api::chat_response_stream_items::snapshot::StreamAssembly;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{
    AssemblyError, Request, stream_chunk_builder,
};
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use super::failure;
use super::sinks::{Hooks, NoCosts, ZeroCounter, hooks};

// The streaming driver hands the end-of-stream assembly its logging context; the helper supplies the builder itself.
pub(crate) struct AssemblyContext {
    now: i64,
    runtime: Runtime,
    hooks: Hooks,
}

impl AssemblyContext {
    pub(crate) fn new(now: i64) -> Result<Self, Box<HelperError>> {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        Ok(Self {
            now,
            runtime,
            hooks: hooks()?,
        })
    }
}

impl StreamAssembly for AssemblyContext {
    fn assemble(
        &self,
        chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<Response>, AssemblyError> {
        let decoded = chunks
            .iter()
            .map(|chunk| {
                let value = serde_json::to_value(chunk)?;
                serde_json::from_value(value).map_err(AssemblyError::from)
            })
            .collect::<Result<Vec<Chunk>, AssemblyError>>()?;
        let counter = ZeroCounter;
        let costs = NoCosts;
        let request = Request {
            chunks: Some(&decoded),
            messages: None,
            count_prompt_tokens: None,
            logging: None,
            costs: &costs,
            counter: &counter,
            metadata: None,
            now: self.now,
        };
        self.runtime
            .block_on(stream_chunk_builder(&request, &self.hooks))
    }
}
