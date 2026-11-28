pub mod auth_admission;
pub mod auth_backend_dispatch;
pub mod auth_cached_sessions;
pub mod auth_common_checks;
pub mod auth_context_sources;
pub mod auth_end_users;
pub mod auth_errors;
pub mod auth_headers;
pub mod auth_identity;
pub mod auth_key_budgets;
pub mod auth_key_lookup;
pub mod auth_key_validation;
pub mod auth_model_budgets;
pub mod auth_model_grants;
pub mod auth_model_lists;
pub mod auth_model_patterns;
pub mod auth_org_lookup;
pub mod auth_policy_support;
pub mod auth_pricing;
pub mod auth_request_models;
pub mod auth_request_policy;
pub mod auth_resource_grants;
pub mod auth_route_patterns;
pub mod auth_route_policy;
pub mod auth_stage_loop;
pub mod auth_tag_budgets;
pub mod auth_tenant_budgets;
pub mod body_buffering;
pub mod budget_admission_handoff;
pub mod central_auth_checks;
pub mod current_spend;
pub mod end_user_identity;
pub mod entity_model_access;
pub mod failures;
pub mod identity_store;
pub mod jwt_db_teams;
pub mod jwt_key_resolution;
pub mod jwt_keys;
pub mod jwt_permissions;
pub mod jwt_provisioning;
pub mod jwt_team_authorization;
pub mod jwt_team_selection;
pub mod jwt_verification;
pub mod key_context;
pub mod key_prefix;
pub mod master_database_auth;
pub mod model_grant_intersection;
pub mod oauth_tokens;
pub mod organization_route_policy;
pub mod request_body_safety;
pub mod route_match;
pub mod team_budget_admission;
pub mod trusted_network;
pub mod virtual_key_routes;

use std::{
    collections::BTreeSet,
    sync::{Arc, PoisonError},
};

use betterlite_auth::auth_request_models::catalog::{EmptyCatalog, RequestModelResolver};
use betterlite_models::{decisions::Json, model_registration::JsonObject};

use crate::auth::auth_identity::AuthBackend::TeamBudgetAdmission;
use crate::auth::auth_identity::{AuthBackend, AuthContext, StageFailure, StageInput};
use crate::auth::auth_key_lookup::ConfiguredMasterKey;
use crate::auth::auth_key_lookup::key_headers::{extract_key, headers_json};
use crate::auth::auth_model_budgets::SharedViews;
use crate::auth::auth_model_patterns::{
    ModelPatternStageInput, max_recurse_depth_for_request, rule_for_request,
};
use crate::auth::auth_request_models::{CoreDecoders, RequestModelStageInput, resolve};
use crate::auth::auth_resource_grants::ResourceGrantStageInput;
use crate::auth::auth_route_policy::stage_input as route_policy_stage_input;
use crate::auth::organization_route_policy::stage_input as org_route_stage_input;
use crate::auth::route_match::PUBLIC_HEALTH_PATHS;
use crate::auth::virtual_key_routes::stage_input as virtual_key_route_stage_input;
use crate::error_body::AuthenticationFailure;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{Request, header};
use axum::middleware::Next;
use axum::response::Response;
use secrecy::{ExposeSecret as _, SecretString};
pub(crate) async fn authenticate(
    State(master_key_state): State<Option<SecretString>>,
    mut request: Request<Body>,
    next: Next,
    failure_response: fn(&StageFailure) -> Response,
) -> Response {
    if PUBLIC_HEALTH_PATHS.contains(&request.uri().path()) {
        return next.run(request).await;
    }
    let Some(master_key) = master_key_state.as_ref() else {
        return next.run(request).await;
    };
    request
        .extensions_mut()
        .insert(ConfiguredMasterKey(master_key.clone()));
    request.extensions_mut().insert(SharedViews::default());
    let pipeline = auth_stage_loop::run_stages(request, master_key, failure_response);
    match Box::pin(pipeline).await {
        Ok((mut staged, authenticated, body)) => {
            let rewritten = rewritten_body(&mut staged, body);
            auth_stage_loop::forward(staged, authenticated, rewritten, next).await
        }
        Err(response) => *response,
    }
}

// a per-model budget fallback rewrites the buffered body, which every downstream handler parses in place of the client's
fn rewritten_body(request: &mut Request<Body>, body: Option<Bytes>) -> Option<Bytes> {
    let snapshots = request
        .extensions()
        .get::<SharedViews>()
        .and_then(|shared| {
            shared
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        });
    let Some(views) = snapshots else {
        return body;
    };
    if let Some(parsed) = views.parsed_body.clone() {
        request.extensions_mut().insert(parsed);
    }
    let rewritten = views.serialized_body.clone().map(Bytes::from).or(body);
    request.extensions_mut().insert(views);
    rewritten
}

fn stage_input(
    backend: AuthBackend,
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    resolver: &Arc<RequestModelResolver>,
    body: Option<&Bytes>,
) -> Result<StageInput, StageFailure> {
    match backend {
        AuthBackend::AuthKeyLookup => Ok(identity_store::lookup_stage_input(request)),
        AuthBackend::MasterKey => Ok(auth_key_lookup::master_stage_input(request, authenticated)),
        AuthBackend::JwtKeys => Ok(jwt_keys::stage_input(request)),
        AuthBackend::TrustedNetwork => trusted_network::stage_input(request, authenticated),
        AuthBackend::AuthRequestPolicy => Ok(StageInput::AuthRequestPolicy(Box::new(
            auth_request_policy::stage_input(request, authenticated, body)?,
        ))),
        AuthBackend::RequestBodySafety => Ok(StageInput::RequestBodySafety(Box::new(
            request_body_safety::stage_input(request, authenticated, body)?,
        ))),
        AuthBackend::JwtPermissions => jwt_permissions::stage_input(request, authenticated, body),
        AuthBackend::AuthVirtualKeyRoutes => virtual_key_route_stage_input(request, authenticated),
        AuthBackend::AuthRequestModels | AuthBackend::AuthModelPatterns => {
            model_pattern_input(backend, request, authenticated, resolver)
        }
        AuthBackend::AuthModelGrants => auth_model_grants::stage_input(request, authenticated),
        AuthBackend::EntityModelAccess => entity_model_access::stage_input(request, authenticated),
        AuthBackend::AuthResourceGrants => resource_grant_stage_input(request, authenticated),
        AuthBackend::AuthTagBudgets => Ok(StageInput::AuthTagBudgets(Box::new(
            auth_tag_budgets::stage_input(request, authenticated)?,
        ))),
        AuthBackend::AuthKeyBudgets => Ok(StageInput::AuthKeyBudgets(Box::new(
            auth_key_budgets::stage_input(request, authenticated)?,
        ))),
        AuthBackend::AuthModelBudgets => auth_model_budgets::stage(request, authenticated, body),
        TeamBudgetAdmission => team_budget_admission::stage_input(request, authenticated),
        AuthBackend::AuthTenantBudgets => auth_tenant_budgets::stage(request, authenticated, body),
        AuthBackend::OrganizationRoutePolicy => org_route_stage_input(request, authenticated, body),
        AuthBackend::AuthRoutePolicy => route_policy_stage_input(request, authenticated, body),
        AuthBackend::AuthOrgLookup => Ok(StageInput::AuthOrgLookup(Box::new(
            auth_org_lookup::stage_input(request, authenticated)?,
        ))),
        AuthBackend::AuthEndUsers => Ok(StageInput::AuthEndUsers(Box::new(
            auth_end_users::stage_input(request, authenticated)?,
        ))),
        AuthBackend::AuthPolicySupport => auth_policy_support::stage(request, authenticated, body),
        AuthBackend::ModelGrantIntersection => model_grant_stage_input(request, authenticated),
    }
}

// the resource grant stage reads the request's rule and database switch, so its input is built here
fn resource_grant_stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let stage = ResourceGrantStageInput {
        context: authenticated
            .cloned()
            .ok_or(AuthenticationFailure::Missing)?,
        rule: auth_resource_grants::rule_for_request(request),
        database: auth_resource_grants::database_for_request(request),
    };
    Ok(StageInput::AuthResourceGrants(Box::new(stage)))
}

fn model_grant_stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    Ok(StageInput::ModelGrantIntersection(Box::new(
        model_grant_intersection::stage_input(request, authenticated)?,
    )))
}

fn model_pattern_input(
    backend: AuthBackend,
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    resolver: &Arc<RequestModelResolver>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let stage = model_stage_input(request, context.clone(), resolver);
    // the request model stage only resolves, so it runs in the shared input builder
    let requested = match resolve(&stage) {
        Ok(requested) => requested,
        Err(error) => {
            tracing::debug!(error = %error, "the request model stage failed");
            return Err(StageFailure::from(AuthenticationFailure::Missing));
        }
    };
    if backend == AuthBackend::AuthRequestModels {
        return Ok(StageInput::AuthRequestModels(Box::new(stage)));
    }
    let rule = rule_for_request(request);
    let depth = max_recurse_depth_for_request(request);
    Ok(StageInput::AuthModelPatterns(Box::new(
        ModelPatternStageInput {
            context,
            requested,
            rule,
            max_recurse_depth: depth,
        },
    )))
}

fn resolver_for(request: &Request<Body>) -> Arc<RequestModelResolver> {
    request
        .extensions()
        .get::<Arc<RequestModelResolver>>()
        .map_or_else(default_resolver, Arc::clone)
}
// stage_input cannot await while holding the request borrow (Request<Body> is Send but not Sync), so the caller awaits the stage prefetch on the owned input
pub(crate) async fn prefetch_stage(input: StageInput) -> Result<StageInput, StageFailure> {
    match input {
        StageInput::AuthKeyLookup(stage) => auth_key_lookup::prefetch(*stage).await,
        StageInput::AuthIdentity(stage) => Box::pin(identity_store::prefetch(*stage)).await,
        StageInput::JwtKeys(stage) => jwt_keys::prefetched(*stage).await,
        StageInput::AuthTagBudgets(stage) => auth_tag_budgets::prefetched(stage).await,
        StageInput::AuthKeyBudgets(stage) => auth_key_budgets::prefetched(stage).await,
        StageInput::AuthModelBudgets(stage) => auth_model_budgets::prefetched(stage).await,
        StageInput::TeamBudgetAdmission(stage) => team_budget_admission::prefetched(stage).await,
        StageInput::AuthTenantBudgets(stage) => auth_tenant_budgets::prefetched(stage).await,
        StageInput::AuthOrgLookup(stage) => auth_org_lookup::prefetched(stage).await,
        StageInput::AuthEndUsers(stage) => auth_end_users::prefetched(stage).await,
        StageInput::ModelGrantIntersection(stage) => StageInput::prefetch_model_grants(stage).await,
        StageInput::AuthModelGrants(stage) => auth_model_grants::prefetched(stage).await,
        StageInput::EntityModelAccess(stage) => entity_model_access::prefetched(stage).await,
        other @ (StageInput::Authenticated(_)
        | StageInput::MasterKey(_)
        | StageInput::TrustedNetwork(_)
        | StageInput::AuthRequestPolicy(_)
        | StageInput::RequestBodySafety(_)
        | StageInput::JwtPermissions(_)
        | StageInput::AuthVirtualKeyRoutes(_)
        | StageInput::AuthRequestModels(_)
        | StageInput::AuthModelPatterns(_)
        | StageInput::AuthResourceGrants(_)
        | StageInput::OrganizationRoutePolicy(_)
        | StageInput::AuthPolicySupport(_)
        | StageInput::AuthRoutePolicy(_)) => Ok(other),
    }
}

fn master_key_stage(
    input: &StageInput,
    master_key: &SecretString,
) -> Result<AuthContext, AuthenticationFailure> {
    let StageInput::MasterKey(presented) = input else {
        return Err(AuthenticationFailure::Missing);
    };
    if let Some(context) = auth_identity::master_key_context(presented.as_ref(), master_key) {
        return Ok(context);
    }
    let failure = match presented.as_ref() {
        Some(key) if !key.expose_secret().is_empty() => AuthenticationFailure::Invalid,
        Some(_) | None => AuthenticationFailure::Missing,
    };
    Err(failure)
}

fn default_resolver() -> Arc<RequestModelResolver> {
    Arc::new(RequestModelResolver {
        catalog: Arc::new(EmptyCatalog),
        decoders: Arc::new(CoreDecoders),
    })
}
fn model_stage_input(
    request: &Request<Body>,
    context: AuthContext,
    resolver: &Arc<RequestModelResolver>,
) -> RequestModelStageInput {
    RequestModelStageInput {
        context,
        route: request.uri().path().to_owned(),
        body: JsonObject::new(),
        headers: Some(headers_json(request.headers())),
        query: Some(query_json(request.uri().query())),
        endpoint_marker: None,
        path_params: BTreeSet::new(),
        team_id: None,
        resolver: Arc::clone(resolver),
    }
}
pub fn parse_key_header(name: &str, header_bytes: &[u8]) -> Option<SecretString> {
    if header_bytes.is_empty() {
        return None;
    }
    let header_value: String = header_bytes.iter().map(|byte| char::from(*byte)).collect();
    let token = if name == header::AUTHORIZATION.as_str() {
        key_prefix::strip_prefix(&header_value).unwrap_or_default()
    } else if name == "x-litellm-api-key" {
        key_prefix::strip_prefix(&header_value).unwrap_or(header_value)
    } else {
        header_value
    };
    Some(SecretString::from(token))
}

const MODEL_QUERY_KEYS: [&str; 2] = ["model", "target_model_names"];

fn query_json(query: Option<&str>) -> JsonObject {
    let mut object = JsonObject::new();
    for (name, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        // LiteLLM reads these two query keys into the model candidates and nothing else
        if MODEL_QUERY_KEYS.contains(&name.to_lowercase().as_str()) {
            object.insert(name.into_owned(), Json::Text(value.into_owned()));
        }
    }
    object
}
