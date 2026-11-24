pub(crate) mod body;
pub(crate) mod error;
pub(crate) mod evidence;
pub(crate) mod findings;
pub(crate) mod lenses;
pub(crate) mod preview;
mod router;
pub(crate) mod runs;
pub(crate) mod workers;

use axum::Router;
use axum::routing::{delete, get, patch, post, put};
use betterlite_db::lens_repository::database::Database;
use betterlite_lens::sources::storage::Storage;

use crate::lens_access::state::LensRoutesState;

// LiteLLM's APIRouter(prefix="/lens") with the static routes declared before /{lens_id}
pub fn router<Db, Store>(state: LensRoutesState<Db, Store>) -> Router
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/lens",
            get(LensRoutesState::<Db, Store>::list_lenses).post(lenses::create_lens),
        )
        .route(
            "/lens/activity/available",
            get(LensRoutesState::<Db, Store>::activity_available),
        )
        .route(
            "/lens/agents",
            get(LensRoutesState::<Db, Store>::list_agents),
        )
        .route("/lens/preview/sample", post(preview::preview_sample))
        .route(
            "/lens/workers/register",
            post(LensRoutesState::<Db, Store>::register_worker),
        )
        .route(
            "/lens/workers/{worker_id}/billing-key",
            put(workers::set_worker_billing),
        )
        .route(
            "/lens/workers/{worker_id}",
            delete(LensRoutesState::<Db, Store>::revoke_worker),
        )
        .route(
            "/lens/{lens_id}",
            put(LensRoutesState::<Db, Store>::update_lens).get(lenses::read_lens),
        )
        .route(
            "/lens/{lens_id}/runs",
            get(LensRoutesState::<Db, Store>::list_runs)
                .post(LensRoutesState::<Db, Store>::run_lens),
        )
        .route(
            "/lens/{lens_id}/runs/{job_id}",
            get(LensRoutesState::<Db, Store>::read_run),
        )
        .route("/lens/{lens_id}/cancel", post(runs::cancel_lens))
        .route(
            "/lens/{lens_id}/findings/{finding_id}",
            patch(findings::update_finding),
        )
        .route(
            "/lens/{lens_id}/executions/{execution_id}",
            get(evidence::evidence_content),
        )
        .with_state(state)
}
