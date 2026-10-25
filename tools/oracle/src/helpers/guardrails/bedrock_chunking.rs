use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_guardrails::bedrock_chunking::{
    ChunkResult, bin_pack, is_input_too_large, merge, nearest_whitespace, split, uses_grounding,
};
use betterlite_guardrails::bedrock_records::ContentItem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    step: Step,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
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

fn encode<Value: Serialize>(value: Value) -> HelperResult {
    let result = serde_json::to_value(value);
    result.map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match &input.step {
            Step::BinPack { content, budget } => encode(bin_pack(content, *budget)),
            Step::Split { content } => encode(split(content)),
            Step::Nearest { text } => encode(nearest_whitespace(text)),
            Step::Merge { chunks } => encode(merge(chunks)),
            Step::Grounding { content } => encode(uses_grounding(content)),
            Step::TooLarge { detail } => encode(is_input_too_large(detail.as_deref())),
        }
    }
}
