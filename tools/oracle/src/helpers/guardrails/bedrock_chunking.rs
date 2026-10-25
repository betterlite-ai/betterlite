use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_guardrails::bedrock_chunking::{
    ChunkResult, ChunkingError, bin_pack, is_input_too_large, merge, nearest_whitespace, split,
    uses_grounding,
};
use betterlite_guardrails::bedrock_records::ContentItem;
use serde::Deserialize;
use serde_json::to_value;

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

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let outcome: Result<_, ChunkingError> = match &input.step {
            Step::BinPack { content, budget } => Ok(to_value(bin_pack(content, *budget))),
            Step::Split { content } => split(content).map(to_value),
            Step::Nearest { text } => Ok(to_value(nearest_whitespace(text))),
            Step::Merge { chunks } => merge(chunks).map(to_value),
            Step::Grounding { content } => Ok(to_value(uses_grounding(content))),
            Step::TooLarge { detail } => Ok(to_value(is_input_too_large(detail.as_deref()))),
        };
        let (class, message) = match outcome {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) => ("SerializationError", error.to_string()),
            Err(error) => ("ChunkingError", error.to_string()),
        };
        Err(Box::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        }))
    }
}
