use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

mod a2a;
mod emit;
mod flags;
mod items;
mod resolution;
mod support;
mod transform;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    #[serde(rename = "a2a_error_chunk")]
    A2aErrorChunk(a2a::ErrorRequest),
    #[serde(rename = "a2a_request_id")]
    A2aRequestId(a2a::RequestIdRequest),
    #[serde(rename = "chunk_predicates")]
    ChunkPredicates(items::PredicateRequest),
    #[serde(rename = "endpoint_resolution")]
    EndpointResolution(resolution::ResolutionRequest),
    #[serde(rename = "http_error_emission")]
    HttpErrorEmission(emit::EmissionRequest),
    #[serde(rename = "redundant_scan")]
    RedundantScan(flags::ScanRequest),
    #[serde(rename = "streaming_flag")]
    StreamingFlag(flags::FlagRequest),
    #[serde(rename = "tool_call_passthrough")]
    ToolCallPassthrough(items::PassthroughRequest),
    #[serde(rename = "transform_chunk")]
    TransformChunk(transform::Request),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::A2aErrorChunk(request) => a2a::error_chunk(request),
            Input::A2aRequestId(request) => a2a::request_id(request),
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
