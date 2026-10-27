use std::collections::BTreeMap;

use super::attributes::Attributes;
use super::model::{Event, Inputs, Payload, Response, Span, SpanKind, Status};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("MLflow operation failed: {0}")]
    Operation(String),
}

pub trait Client: Send + Sync {
    type Data: Payload;

    fn active_span(&self) -> Result<Option<Span>, ClientError>;
    fn start_trace(&self, request: StartRequest<Self::Data>) -> Result<Span, ClientError>;
    fn start_span(
        &self,
        request: StartRequest<Self::Data>,
        parent: &Span,
    ) -> Result<Span, ClientError>;
    fn add_event(&self, span: &Span, event: Event) -> Result<(), ClientError>;
    fn chat_available(&self) -> bool;
    fn set_chat_messages(&self, span: &Span, messages: Vec<Self::Data>) -> Result<(), ClientError>;
    fn set_chat_tools(&self, span: &Span, tools: Vec<Self::Data>) -> Result<(), ClientError>;
    fn end_trace(
        &self,
        span: &Span,
        request: EndRequest<'_, Self::Data>,
    ) -> Result<(), ClientError>;
    fn end_span(&self, span: &Span, request: EndRequest<'_, Self::Data>)
    -> Result<(), ClientError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartRequest<Data> {
    pub name: String,
    pub kind: SpanKind,
    pub inputs: Inputs<Data>,
    pub attributes: Attributes<Data>,
    pub tags: BTreeMap<String, String>,
    pub start_time_ns: i64,
}

#[derive(Debug)]
pub struct EndRequest<'response, Data> {
    pub outputs: &'response Response<Data>,
    pub status: Status,
    pub end_time_ns: i64,
}
