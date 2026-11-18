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
#[error("the semantic router could not be built: {source}")]
pub struct RouterBuildError<E>
where
    E: Error + Send + Sync + 'static,
{
    #[source]
    source: E,
}

// the caller loads the routes first, so no file is read on the executor while the router is built
pub async fn build_router<B>(
    builder: &B,
    routes: Vec<Route>,
    global_threshold: f64,
) -> Result<B::Router, RouterBuildError<B::Error>>
where
    B: SemanticRouterBuilder,
{
    builder
        .build(routes, global_threshold)
        .await
        .map_err(|source| RouterBuildError { source })
}

#[derive(Debug, thiserror::Error)]
pub enum RouterSetupError<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Load(#[from] RouteLoadError),
    #[error(transparent)]
    Build(#[from] RouterBuildError<E>),
}

// one entry point for the caller, which loads the routes first so a load failure never reaches the builder
pub async fn load_and_build_router<B>(
    builder: &B,
    sources: &RouteSources,
) -> Result<B::Router, RouterSetupError<B::Error>>
where
    B: SemanticRouterBuilder,
{
    let routes = build_routes(sources)?;
    build_router(builder, routes, sources.global_threshold)
        .await
        .map_err(RouterSetupError::Build)
}
