pub mod body;
pub(crate) mod error;
pub(crate) mod evidence;
pub(crate) mod findings;
pub(crate) mod lenses;
pub mod preview;
mod router;
pub(crate) mod runs;
pub(crate) mod tests;
pub mod workers;

use axum::Router;
use axum::routing::{delete, get, patch, post, put};
use betterlite_db::lens_repository::database::Database;
use betterlite_lens::sources::storage::Storage;

use crate::lens_access::state::LensRoutesState as LensState;

// LiteLLM's APIRouter(prefix="/lens") with the static routes declared before /{lens_id}
pub fn router<Db, Store>(state: LensState<Db, Store>) -> Router
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let lens_collection = get(LensState::<Db, Store>::list_lenses).post(lenses::create_lens);
    let activity_available = get(LensState::<Db, Store>::activity_available);
    let agent_list = get(LensState::<Db, Store>::list_agents);
    let sample_preview = post(preview::preview_sample);
    let worker_register = post(workers::register_worker);
    let worker_billing = put(workers::billing::set_worker_billing);
    let worker_revoke = delete(workers::revoke_worker);
    let lens_detail = put(LensState::<Db, Store>::update_lens).get(lenses::read_lens);
    let run_collection = get(runs::list_runs).post(runs::run_lens);
    let run_detail = get(runs::read::read_run);
    let lens_cancel = post(runs::cancel::cancel_lens);
    let finding_patch = patch(findings::update_finding);
    let execution_content = get(evidence::evidence_content);
    Router::new()
        .route("/lens", lens_collection)
        .route("/lens/activity/available", activity_available)
        .route("/lens/agents", agent_list)
        .route("/lens/preview/sample", sample_preview)
        .route("/lens/workers/register", worker_register)
        .route("/lens/workers/{worker_id}/billing-key", worker_billing)
        .route("/lens/workers/{worker_id}", worker_revoke)
        .route("/lens/{lens_id}", lens_detail)
        .route("/lens/{lens_id}/runs", run_collection)
        .route("/lens/{lens_id}/runs/{job_id}", run_detail)
        .route("/lens/{lens_id}/cancel", lens_cancel)
        .route("/lens/{lens_id}/findings/{finding_id}", finding_patch)
        .route(
            "/lens/{lens_id}/executions/{execution_id}",
            execution_content,
        )
        .with_state(state)
}
