mod claim;
mod content;
mod credential;
mod error;
mod evidence;
mod lease;
mod merge;
mod model;
mod progress;
mod result;
mod router;
mod sample;
pub(crate) mod tests;

use axum::Router;
use axum::routing::{get, post};

use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage, NoStorage};
use crate::lens_worker_routes::model::Inference;

// B4503's routes state plus the analysis bundle lens_inference::analyze reaches, both injected by the mount
pub(crate) struct WorkerRoutesState<Db, Store = NoStorage> {
    routes: LensRoutesState<Db, Store>,
    inference: Inference,
}

impl<Db, Store> WorkerRoutesState<Db, Store> {
    pub(crate) fn new(routes: LensRoutesState<Db, Store>, inference: Inference) -> Self {
        Self { routes, inference }
    }
}

impl<Db, Store: Clone> Clone for WorkerRoutesState<Db, Store> {
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
            inference: self.inference.clone(),
        }
    }
}

// LiteLLM's APIRouter(prefix="/lens") worker routes, none of them behind user_api_key_auth
pub(crate) fn router<Db, Store>(state: WorkerRoutesState<Db, Store>) -> Router
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let claim_route = post(claim::claim);
    let progress_route = post(progress::progress);
    let heartbeat_route = post(progress::heartbeat);
    let sample_route = get(sample::sample);
    let content_route = get(content::content);
    let model_route = post(model::model);
    let result_route = post(result::result);
    Router::new()
        .route("/lens/worker/claim", claim_route)
        .route("/lens/worker/{lens_id}/{job_id}/progress", progress_route)
        .route("/lens/worker/{lens_id}/{job_id}/heartbeat", heartbeat_route)
        .route("/lens/worker/{lens_id}/{job_id}/sample", sample_route)
        .route("/lens/worker/{lens_id}/{job_id}/content", content_route)
        .route("/lens/worker/{lens_id}/{job_id}/model", model_route)
        .route("/lens/worker/{lens_id}/{job_id}/result", result_route)
        .with_state(state)
}
