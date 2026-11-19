pub mod encryption;
pub mod error;
pub mod lookup;
pub mod masking;
pub mod merge;
pub mod registry;
pub mod request;
pub mod state;
pub mod store;

mod context;
mod create;
mod delete;
mod patch;
mod read;
mod wire;

pub mod migration;
pub mod rotation;

use axum::Router;
use axum::routing::{get, patch};

pub fn router(state: state::CredentialState) -> Router {
    Router::new()
        .route(
            "/credentials",
            get(read::list_credentials).post(create::create_credential),
        )
        .route(
            "/credentials/by_name/{*credential_name}",
            get(read::get_credential_by_name),
        )
        .route(
            "/credentials/by_model/{model_id}",
            get(read::get_credential_by_model),
        )
        .route(
            "/credentials/{*credential_name}",
            patch(patch::update_credential).delete(delete::delete_credential),
        )
        .with_state(state)
}

mod tests;
