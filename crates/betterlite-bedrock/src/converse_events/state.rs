use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::converse_events::ChunkProjection;
use crate::converse_events::ConverseStreamState;
use crate::converse_events::dispatch::{self, BaseOutcome};
use crate::converse_events::error::ConverseEventError;
use crate::converse_events::projection::{
    assemble_converse, initialize_response_id, project_converse,
};
use crate::converse_events::response::ConverseChunk;
use crate::converse_events::wire;

impl ConverseStreamState {
    pub fn converse_chunk_parser(
        &mut self,
        chunk: &Json,
    ) -> Result<ConverseChunk, ConverseEventError> {
        self.project_event(chunk)
            .map_err(|source| ConverseEventError::Streaming(Box::new(source)))
    }

    fn project_event(&mut self, chunk: &Json) -> Result<ConverseChunk, ConverseEventError> {
        let fields = wire::object(chunk, "converse event")?;
        initialize_response_id(self, fields)?;
        let index = wire::content_index(fields.get("contentBlockIndex"))?;
        let projection = project_converse(self, fields, index)?;
        if projection
            .parts
            .thinking_blocks
            .as_ref()
            .is_some_and(|blocks| !blocks.is_empty())
        {
            self.thinking_ran = true;
        }
        Ok(assemble_converse(self, fields, projection))
    }

    pub fn chunk_parser(&mut self, chunk: &Json) -> Result<ChunkProjection, ConverseEventError> {
        let fields = wire::object(chunk, "provider chunk")?;
        match dispatch::base_chunk(chunk, fields, &self.model)? {
            BaseOutcome::Converse(payload) => Ok(ChunkProjection::Converse(
                self.converse_chunk_parser(payload)?,
            )),
            BaseOutcome::Generic {
                text,
                is_finished,
                finish_reason,
            } => Ok(ChunkProjection::Generic(GenericChunk {
                text: text.to_owned(),
                tool_use: None,
                is_finished,
                finish_reason: finish_reason.to_owned(),
                usage: None,
                index: Some(0),
                provider_specific_fields: None,
            })),
        }
    }

    pub fn check_empty_tool_call_args(&self) -> Result<bool, ConverseEventError> {
        let Some(first_value) = self.content_blocks.first() else {
            return Ok(false);
        };
        let first_block = wire::object(first_value, "content_blocks[0]")?;
        if !first_block.contains_key("toolUse") {
            return Ok(false);
        }
        let mut arguments = String::new();
        for value in &self.content_blocks {
            let block = wire::object(value, "content_blocks")?;
            if let Some(tool) = block.get("toolUse") {
                let tool_block = wire::object(tool, "content_blocks.toolUse")?;
                arguments.push_str(wire::required_text(
                    tool_block,
                    "input",
                    "content_blocks.toolUse.input",
                )?);
            }
        }
        Ok(arguments.is_empty())
    }
}
