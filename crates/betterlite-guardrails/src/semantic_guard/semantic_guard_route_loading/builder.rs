use std::error::Error;

use async_trait::async_trait;

use super::build_routes;
use super::error::RouteLoadError;
use super::route::Route;
use super::sources::RouteSources;

#[async_trait]
pub trait SemanticRouterBuilder: Send + Sync {
    type Router: Send;

    type Error: Error + Send + Sync + 'static;

    async fn build(
        &self,
        routes: Vec<Route>,
        global_threshold: f64,
    ) -> Result<Self::Router, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum RouterBuildError<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Load(#[from] RouteLoadError),
    #[error("the semantic router could not be built: {source}")]
    Build {
        #[source]
        source: E,
    },
}

pub async fn build_router<B>(
    builder: &B,
    sources: &RouteSources,
) -> Result<B::Router, RouterBuildError<B::Error>>
where
    B: SemanticRouterBuilder,
{
    let routes = build_routes(sources)?;
    builder
        .build(routes, sources.global_threshold)
        .await
        .map_err(|source| RouterBuildError::Build { source })
}
