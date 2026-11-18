use std::error::Error;

use async_trait::async_trait;

// the semantic_router answer's route, whose score semantic_router may leave unset
#[derive(Debug, Clone, PartialEq)]
pub struct RouteChoice {
    pub name: Option<String>,
    pub similarity_score: Option<f64>,
}

// the built router answers one query, so the guardrail never owns an embedding service of its own
#[async_trait]
pub trait SemanticRouteScorer: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn score(&self, text: &str) -> Result<Vec<RouteChoice>, Self::Error>;
}
