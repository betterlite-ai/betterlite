use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_guardrails::bedrock_chunking::{
    ChunkResult, ChunkingError, bin_pack, is_input_too_large, merge, nearest_whitespace, split, uses_grounding,
};
use betterlite_guardrails::bedrock_records::ContentItem;
use serde::{Deserialize, Serialize};

// Externally tagged: an internally tagged enum buffers its input, and the records' raw extension fields cannot be read from that buffer.
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    step: Step,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    BinPack {
        content: Vec<ContentItem>,
        budget: usize,
    },
    Split {
        content: Vec<ContentItem>,
    },
    Nearest {
        text: String,
    },
    Merge {
        chunks: Vec<ChunkResult>,
    },
    Grounding {
        content: Vec<ContentItem>,
    },
    TooLarge {
        detail: Option<String>,
    },
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn encode<Value>(value: Value) -> HelperResult
where
    Value: Serialize,
{
    serde_json::to_value(value).map_err(|error| failure("SerializationError", error.to_string()))
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let chunking = |error: ChunkingError| failure("ChunkingError", error.to_string());
        match &input.step {
            Step::BinPack { content, budget } => encode(bin_pack(content, *budget)),
            Step::Split { content } => encode(split(content).map_err(chunking)?),
            Step::Nearest { text } => encode(nearest_whitespace(text)),
            Step::Merge { chunks } => encode(merge(chunks).map_err(chunking)?),
            Step::Grounding { content } => encode(uses_grounding(content)),
            Step::TooLarge { detail } => encode(is_input_too_large(detail.as_deref())),
        }
    }
}
