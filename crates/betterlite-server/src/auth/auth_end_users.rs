use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::proxy_exception::ProxyException;
use rust_decimal::Decimal;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// What a request carries about its end user: B880's resolved id and the key's two budget fields.
#[derive(Clone, Debug, Default)]
pub struct EndUserRule {
    pub end_user_id: Option<String>,
    pub key_end_user_budget_id: Option<String>,
    pub token_end_user_max_budget: Option<Decimal>,
    pub lookup: Option<Arc<dyn EndUserLookup>>,
}

// LiteLLM's get_end_user_object and _check_end_user_budget over whatever dependencies the proxy wired.
#[async_trait]
pub trait EndUserLookup: Send + Sync + fmt::Debug {
    async fn end_user_object(
        &self,
        end_user_id: &str,
        key_end_user_budget_id: Option<&str>,
        token_end_user_max_budget: Option<Decimal>,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserFailure>;

    async fn check_budget(
        &self,
        end_user: &LiteLLMEndUserTable,
        route: &str,
    ) -> Result<(), EndUserFailure>;
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum EndUserFailure {
    #[error("{}", .0.message.as_deref().unwrap_or("end-user budget exceeded"))]
    Budget(BudgetExceededDetails),
    #[error("{0}")]
    Failure(String),
}

#[derive(Clone, Debug)]
pub struct EndUserStageInput {
    pub context: AuthContext,
    pub rule: Arc<EndUserRule>,
    pub route: String,
    pub database: bool,
}

pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<EndUserRule> {
    request
        .extensions()
        .get::<Arc<EndUserRule>>()
        .map_or_else(|| Arc::new(EndUserRule::default()), Arc::clone)
}

pub(crate) fn database_for_request(request: &Request<Body>) -> bool {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .is_some_and(|state| state.auth_end_users.database)
}

// the lookup is asynchronous, so the stage keeps its own input and the caller awaits the prefetch
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<EndUserStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    Ok(EndUserStageInput {
        context,
        rule: rule_for_request(request),
        route: request.uri().path().to_owned(),
        database: database_for_request(request),
    })
}

pub(crate) async fn prefetched(stage: Box<EndUserStageInput>) -> Result<StageInput, StageFailure> {
    prefetch(&stage).await.map_err(StageFailure::EndUsers)?;
    Ok(StageInput::AuthEndUsers(stage))
}

// LiteLLM's get_end_user_object never runs without a database, and an unusable lookup proves nothing.
pub(crate) async fn prefetch(stage: &EndUserStageInput) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let Some(lookup) = rule.lookup.as_deref() else {
        return Ok(());
    };
    if !stage.database {
        return Ok(());
    }
    let Some(end_user_id) = rule.end_user_id.as_deref() else {
        return Ok(());
    };
    let end_user = lookup
        .end_user_object(
            end_user_id,
            rule.key_end_user_budget_id.as_deref(),
            rule.token_end_user_max_budget,
        )
        .await
        .map_err(|failure| Box::new(exception(&failure)))?;
    let Some(found) = end_user else {
        return Ok(());
    };
    lookup
        .check_budget(&found, &stage.route)
        .await
        .map_err(|failure| Box::new(exception(&failure)))
}

// B757's mapping: a budget rejection is budget_exceeded with a null param and the configured status.
fn exception(failure: &EndUserFailure) -> ProxyException {
    let mapped = match failure {
        EndUserFailure::Budget(details) => AuthFailure::BudgetExceeded {
            message: details
                .message
                .clone()
                .unwrap_or_else(|| default_message(details)),
            status_code: Some(details.status_code),
            llm_provider: details.provider.clone(),
        },
        EndUserFailure::Failure(message) => AuthFailure::Other {
            message: message.clone(),
            param: None,
        },
    };
    as_proxy_exception(&mapped).exception().clone()
}

fn default_message(details: &BudgetExceededDetails) -> String {
    format!(
        "Budget has been exceeded! Current cost: {}, Max budget: {}",
        details.current_cost, details.max_budget
    )
}
