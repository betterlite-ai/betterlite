use axum::extract::{Extension, RawQuery, State};
use axum::response::{IntoResponse as _, Response};

use super::LogListState;
use super::execute::execute;
use crate::auth::auth_identity::AuthContext;
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::tracing_routes::router::unauthenticated;

// ui_view_spend_logs: one handler behind both decorated routes, which the layer tells apart
pub(super) async fn ui_view_spend_logs(
    State(state): State<LogListState>,
    Extension(route): Extension<SpendLogRoute>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    match execute(&state, route, &context.principal, raw_query.as_deref()).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}
