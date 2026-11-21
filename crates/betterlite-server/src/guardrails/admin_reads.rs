mod caller;
mod error;
mod forms;
mod info;
mod list;
mod list_config;
pub(crate) mod masking;
mod projection;
mod provider_modes;
mod settings;
pub mod state;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;

use self::forms::provider_specific_params;
use self::info::guardrail_info;
use self::list::list_guardrails_v2;
use self::list_config::list_guardrails;
use self::settings::guardrail_ui_settings;
use self::state::ReadState;

// B8667 mounts these routes and owns their HTTP oracle cases
pub fn routes<R>(state: ReadState<R>) -> Router
where
    R: GuardrailRows + 'static,
{
    Router::new()
        .route("/guardrails/list", get(list_guardrails::<R>))
        .route("/v2/guardrails/list", get(list_guardrails_v2::<R>))
        .route(
            "/guardrails/ui/add_guardrail_settings",
            get(guardrail_ui_settings::<R>),
        )
        .route(
            "/guardrails/ui/provider_specific_params",
            get(provider_specific_params::<R>),
        )
        .route("/guardrails/{guardrail_id}", get(guardrail_info::<R>))
        .route("/guardrails/{guardrail_id}/info", get(guardrail_info::<R>))
        .with_state(Arc::new(state))
}
