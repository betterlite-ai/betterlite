pub mod config;
pub mod error;
pub mod records;
mod tests;

use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use records::{
    CompactRequest, CreateRequest, IdRequest, InputItemsRequest, Params, PreparedRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesRequest {
    Create(CreateRequest),
    Stream(CreateRequest),
    Get(IdRequest),
    Delete(IdRequest),
    InputItems(InputItemsRequest),
    Cancel(IdRequest),
    Compact(CompactRequest),
}

#[derive(Debug, Clone)]
pub enum ResponsesResult {
    Create(Box<ResponsesApiResponse>),
    Stream(Box<ResponsesAPIStreamingResponse>),
    Get(Box<ResponsesApiResponse>),
    Delete(Box<DeleteResponseResult>),
    InputItems(Params),
    Cancel(Box<ResponsesApiResponse>),
    Compact(Box<ResponsesApiResponse>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesPrepared {
    Create(PreparedRequest),
    Stream(PreparedRequest),
    Get(PreparedRequest),
    Delete(PreparedRequest),
    InputItems(PreparedRequest),
    Cancel(PreparedRequest),
    Compact(PreparedRequest),
}
