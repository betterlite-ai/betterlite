use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmbeddingRerank;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmbeddingRerankRequest;
pub struct EmbeddingRerankResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmbeddingRerankPrepared;

impl Adapter for EmbeddingRerank {
    const NAME: &'static str = "embedding_rerank";
}
