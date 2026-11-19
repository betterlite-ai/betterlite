pub mod connections;
pub mod repositories;

use axum::Router;
use axum::routing::{get, post};

use crate::roi_github::transport::Transport;

use super::state::RoiConnectionsState;

pub fn router<T>(state: RoiConnectionsState<T>) -> Router
where
    T: Transport + Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            repositories::REPOSITORIES_PATH,
            get(repositories::list_repositories::<T>),
        )
        .route(
            connections::CONNECTIONS_TEST_PATH,
            post(connections::test_connections::<T>),
        )
        .with_state(state)
}
