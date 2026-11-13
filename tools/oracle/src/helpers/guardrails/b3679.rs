use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

mod emit;
mod flags;
mod items;
mod resolution;
mod support;
mod transform;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ChunkPredicates(items::PredicateRequest),
    EndpointResolution(resolution::ResolutionRequest),
    HttpErrorEmission(emit::EmissionRequest),
    RedundantScan(flags::ScanRequest),
    StreamingFlag(flags::FlagRequest),
    ToolCallPassthrough(items::PassthroughRequest),
    TransformChunk(transform::Request),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ChunkPredicates(request) => Ok(items::predicates(request)),
            Input::EndpointResolution(request) => resolution::resolve(request),
            Input::HttpErrorEmission(request) => emit::emission(request),
            Input::RedundantScan(request) => Ok(flags::redundant(request)),
            Input::StreamingFlag(request) => flags::streaming_flag(request),
            Input::ToolCallPassthrough(request) => items::passthrough(request),
            Input::TransformChunk(request) => transform::run(request),
        }
    }
}
