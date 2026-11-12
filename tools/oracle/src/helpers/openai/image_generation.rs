use serde::Deserialize;
use serde_json::Value;

use crate::helpers::HelperResult;

mod input;
mod request;
mod support;
mod tests;
mod transform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Dalle2Parameters,
    Dalle3QualityStyle,
    GptImageUsage,
    B64JsonAndUrl,
    MissingCreated,
    ImageStatusError,
    UnsupportedImageParam,
    ImageRequestOverlay,
    ImageRawResponseHeaders,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    probes: Vec<input::Probe>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .probes
            .iter()
            .map(|probe| match input.function {
                Function::Dalle2Parameters
                | Function::Dalle3QualityStyle
                | Function::GptImageUsage
                | Function::B64JsonAndUrl
                | Function::MissingCreated
                | Function::UnsupportedImageParam => transform::value(probe, input.function),
                Function::ImageStatusError
                | Function::ImageRequestOverlay
                | Function::ImageRawResponseHeaders => request::value(probe),
            })
            .collect::<Result<Vec<Value>, Box<crate::helpers::HelperError>>>()
            .map(Value::Array)
    }
}
