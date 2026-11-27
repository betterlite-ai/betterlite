use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use tokio_util::task::TaskTracker;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_end_users::defaults::KeyEndUserDefaults;
use betterlite_auth::auth_identity::records::EndUserIdentity;
use betterlite_auth::end_user_identity::resolve::{
    EndUserIdResolver, ResolveError, ResolveRequest,
};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_logging::end_user::extraction::{IdSettings, request_end_user_id};

use crate::auth::auth_end_users::{self, EndUserStageInput};
use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::request_body_safety::request_object;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// The resolver for this request: what the proxy wired, or else the one the state builds from its config.
#[derive(Clone, Debug, Default)]
pub struct EndUserIdentityRule {
    pub resolver: Option<Arc<EndUserIdResolver>>,
}

#[derive(Clone)]
pub struct EndUserIdentityStageInput {
    pub context: AuthContext,
    pub raw_end_user_id: Option<String>,
    pub rule: Arc<EndUserIdentityRule>,
    pub database: bool,
    pub state: Option<Arc<ServerState>>,
    pub tasks: Option<TaskTracker>,
}

// The rule the proxy wired stays authoritative, and without one the state supplies the production resolver.
fn rule_for_request(request: &Request<Body>) -> Arc<EndUserIdentityRule> {
    if let Some(rule) = request.extensions().get::<Arc<EndUserIdentityRule>>() {
        return Arc::clone(rule);
    }
    Arc::new(EndUserIdentityRule::default())
}

fn database_for_request(request: &Request<Body>) -> bool {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .is_some_and(|state| state.end_user_identity.database)
}

// B10050's request_end_user_id over the parsed body and the request headers
fn extracted_id(request: &Request<Body>, body: Option<&Bytes>) -> Option<String> {
    let slot = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(|state| &state.end_user_identity)?;
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let bytes = body.cloned().unwrap_or_default();
    let object = request_object(&bytes, content_type).unwrap_or_default();
    let settings = IdSettings {
        request_headers: &slot.request_headers,
        validate_end_user_id_in_db: slot.validate_end_user_id_in_db,
    };
    request_end_user_id(&object, Some(request.headers()), &settings)
}

// the dispatcher builds this stage's variant here, as organization_route_policy does for its own
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<StageInput, StageFailure> {
    Ok(StageInput::AuthEndUserIdentity(Box::new(
        EndUserIdentityStageInput {
            context: authenticated
                .cloned()
                .ok_or(AuthenticationFailure::Missing)?,
            raw_end_user_id: extracted_id(request, body),
            rule: rule_for_request(request),
            database: database_for_request(request),
            state: request.extensions().get::<Arc<ServerState>>().cloned(),
            tasks: request.extensions().get::<TaskTracker>().cloned(),
        },
    )))
}

// the validation is asynchronous, so the stage keeps its own input and the caller awaits the prefetch
pub(crate) async fn prefetch(
    stage: &mut EndUserIdentityStageInput,
) -> Result<(), Box<ProxyException>> {
    let key_end_user_budget_id = stage
        .context
        .principal
        .source_key
        .0
        .as_ref()
        .and_then(|key| KeyEndUserDefaults::of_key(key).key_end_user_budget_id);
    let production_resolver = installed_resolver(stage).await;
    let resolver = stage
        .rule
        .resolver
        .as_deref()
        .or(production_resolver.as_deref());
    let validated_id = match resolver {
        Some(identity) if stage.database => identity
            .resolve(&ResolveRequest {
                raw_end_user_id: stage.raw_end_user_id.as_deref(),
                key_end_user_budget_id: key_end_user_budget_id.as_deref(),
            })
            .await
            .map_err(|error| Box::new(exception(&error)))?,
        // no database client is LiteLLM's prisma_client None, where the supplied id passes through
        Some(_) | None => stage.raw_end_user_id.clone(),
    };
    stage.context.principal.end_user = validated_id.map(|id| EndUserIdentity { id });
    Ok(())
}

async fn installed_resolver(stage: &EndUserIdentityStageInput) -> Option<Arc<EndUserIdResolver>> {
    if let (None, true, Some(state), Some(tasks)) = (
        stage.rule.resolver.as_ref(),
        stage.database,
        stage.state.clone(),
        stage.tasks.as_ref(),
    ) {
        let background = tasks.clone();
        tasks
            .spawn_blocking(move || state.end_user_identity.resolver(&state, &background))
            .await
            .unwrap_or_else(|error| {
                tracing::error!(%error, "the end-user identity resolver initialization failed");
                None
            })
    } else {
        None
    }
}

// the budget stage that follows the validation takes its variant here too, since it reads the id this stage kept
pub(crate) fn budget_stage(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    Ok(StageInput::AuthEndUsers(Box::new(
        auth_end_users::stage_input(request, authenticated).map_err(StageFailure::from)?,
    )))
}

// LiteLLM reads the id this stage validated, and none other, when the end-user budget lookup follows
pub(crate) async fn prefetch_budget(
    mut stage: Box<EndUserStageInput>,
) -> Result<StageInput, StageFailure> {
    Arc::make_mut(&mut stage.rule).end_user_id = stage
        .context
        .principal
        .end_user
        .as_ref()
        .map(|identity| identity.id.clone());
    auth_end_users::prefetched(stage).await
}

// LiteLLM lets a cache failure escape the auth path as an unhandled internal server error.
fn exception(error: &ResolveError) -> ProxyException {
    tracing::error!(error = %error, "the end-user id validation failed");
    ProxyException::new(ProxyExceptionInit {
        message: "Internal server error".to_owned(),
        error_type: "internal_server_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(500_i64),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

impl fmt::Debug for EndUserIdentityStageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndUserIdentityStageInput")
            .field("context", &self.context)
            .field("raw_end_user_id", &self.raw_end_user_id)
            .field("rule", &self.rule)
            .field("database", &self.database)
            .field("state", &self.state.is_some())
            .field("tasks", &self.tasks.is_some())
            .finish()
    }
}
