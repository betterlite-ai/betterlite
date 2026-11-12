use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VertexAi;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VertexAiRequest;
pub struct VertexAiResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VertexAiPrepared;

impl Adapter for VertexAi {
    const NAME: &'static str = "vertex_ai";
}
