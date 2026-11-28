pub mod body;
mod create;
mod delete;
mod error;
mod patch;
mod replace;
mod response;
mod review;
pub mod review_rows;
mod sync;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::extract::Extension;
use axum::routing::{post, put};
use betterlite_auth::auth_identity::roles::Role;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_guardrails::guardrail_startup::runtime::GuardrailRuntime;

use self::create::create_guardrail;
use self::delete::delete_guardrail;
use self::error::MutationError;
use self::patch::patch_guardrail;
use self::replace::replace_guardrail;
use self::review::{Decision, review_submission};
use self::review_rows::ReviewRows;
use crate::auth::auth_identity::AuthContext;

pub struct MutationState<R: GuardrailRows, S: ReviewRows> {
    pub database: Option<MutationDatabase<R, S>>,
}

pub struct MutationDatabase<R: GuardrailRows, S: ReviewRows> {
    pub runtime: Arc<GuardrailRuntime<R>>,
    pub review_rows: Arc<S>,
}

// B8667 mounts these routes and owns their HTTP oracle cases
pub fn routes<R, S>(state: MutationState<R, S>) -> Router
where
    R: GuardrailRows + 'static,
    S: ReviewRows + 'static,
{
    Router::new()
        .route("/guardrails", post(create_guardrail::<R, S>))
        .route(
            "/guardrails/{guardrail_id}",
            put(replace_guardrail::<R, S>)
                .patch(patch_guardrail::<R, S>)
                .delete(delete_guardrail::<R, S>),
        )
        .route(
            "/guardrails/submissions/{guardrail_id}/approve",
            post(review_submission::<R, S>).layer(Extension(Decision::Approve)),
        )
        .route(
            "/guardrails/submissions/{guardrail_id}/reject",
            post(review_submission::<R, S>).layer(Extension(Decision::Reject)),
        )
        .with_state(Arc::new(state))
}

// FastAPI solves the key check before the body, so a missing key answers 401 even for a bad body
fn authenticated(caller: Option<Extension<AuthContext>>) -> Result<AuthContext, MutationError> {
    match caller {
        Some(Extension(context)) => Ok(context),
        None => Err(MutationError::Unauthenticated),
    }
}

// B732's rule: only the exact proxy_admin role counts, so a proxy_admin_viewer is refused
fn require_admin(context: &AuthContext, detail: &'static str) -> Result<(), MutationError> {
    let principal = &context.principal;
    if principal.roles.contains(&Role::PlatformAdmin) {
        return Ok(());
    }
    Err(MutationError::Forbidden { detail })
}
