use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use axum::response::Response;
use betterlite_auth::auth_context_sources::router_settings::TeamRouterSettings;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::RouteValue;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{
    RequestBodyInput, RequestBodyResult, rewrite_request_model,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;

use crate::api::request_body::read_request_body;
use crate::auth::auth_errors::FailureContext;
use crate::auth::auth_identity::{AuthBackend, StageFailure, dispatch};
use crate::auth::body_buffering::{BufferError, max_body_bytes, read_body};
use crate::auth::failures::{body_too_large_response, unreadable_body_response};
use crate::auth::{headers_json, prefetch_stage, query_json};
use crate::state::ServerState;

use super::PreAuthenticated;
use super::key_lookup_stage_input;
use super::selection::{Selection, select_model};

#[derive(Clone, Default)]
pub struct RewriteRule {
    pub key_aliases: Option<Json>,
    pub team_aliases: Option<Json>,
    pub key_settings: Option<Json>,
    pub team_id: Option<String>,
    pub teams: Option<Arc<dyn TeamRouterSettings>>,
    pub endpoint_marker: Option<bool>,
    pub path_params: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Markers {
    pub normalized: bool,
    pub resolved: bool,
    pub client_model: Option<String>,
}

#[async_trait]
impl TeamRouterSettings for RewriteRule {
    async fn router_settings(
        &self,
        team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError> {
        match self.teams.as_deref() {
            Some(teams) => teams.router_settings(team_id).await,
            None => Ok(None),
        }
    }
}

pub(crate) async fn prepare(
    mut request: Request<Body>,
    master_key: &SecretString,
    failure_response: fn(&StageFailure) -> Response,
) -> Result<Request<Body>, Box<Response>> {
    if !is_llm_api_route(&RouteValue::Text(request.uri().path())) {
        return Ok(request);
    }
    // Authenticate before buffering with the same lookup the stage loop runs first, and let that stage forward this identity.
    let context = match prefetch_stage(key_lookup_stage_input(&request))
        .await
        .and_then(|input| dispatch(AuthBackend::AuthKeyLookup, &input, master_key))
    {
        Ok(context) => context,
        // the stage loop answers its failures through the auth error hook, so this early lookup does the same
        Err(failure) => {
            let failed = FailureContext::of(&request, None);
            return Err(Box::new(failed.respond(&failure, failure_response).await));
        }
    };
    request.extensions_mut().insert(PreAuthenticated(context));
    let Some(state) = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(Arc::clone)
    else {
        return Ok(request);
    };
    buffered(&mut request, &state, failure_response).await?;
    Ok(request)
}

async fn buffered(
    request: &mut Request<Body>,
    state: &ServerState,
    failure_response: fn(&StageFailure) -> Response,
) -> Result<(), Box<Response>> {
    let limit = max_body_bytes(request);
    let stream = mem::replace(request.body_mut(), Body::empty());
    let bytes = read_body(stream, limit).await.map_err(|error| {
        Box::new(match error {
            BufferError::TooLarge { observed } => body_too_large_response(observed, limit),
            BufferError::Unreadable(_reason) => unreadable_body_response(),
        })
    })?;
    let mut input = RequestBodyInput {
        route_path: request.uri().path().to_owned(),
        method: request.method().to_string(),
        headers: request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.to_string(), text.to_owned()))
            })
            .collect(),
        body: bytes.to_vec(),
        query_params: BTreeMap::new(),
        client_requested_model: None,
        cached_body: None,
        form_fields: None,
    };
    let rewritten = rewrite(request, state, &mut input)
        .await
        .map_err(|failure| Box::new(failure_response(&failure)))?;
    restore_body(request, bytes, rewritten);
    Ok(())
}

fn restore_body(request: &mut Request<Body>, bytes: Bytes, rewritten: Option<Vec<u8>>) {
    if rewritten.is_some() {
        request.headers_mut().remove(header::CONTENT_LENGTH);
    }
    *request.body_mut() = rewritten.map_or_else(|| Body::from(bytes), Body::from);
}

async fn rewrite(
    request: &mut Request<Body>,
    state: &ServerState,
    input: &mut RequestBodyInput,
) -> Result<Option<Vec<u8>>, StageFailure> {
    let Ok(decoded) = read_request_body(
        input,
        &state.config.sections.request_body,
        &ApiLimits::default(),
    ) else {
        return Ok(None);
    };
    let extensions = request.extensions();
    let rule = extensions
        .get::<Arc<RewriteRule>>()
        .map(Arc::clone)
        .unwrap_or_default();
    let mut markers = extensions.get::<Markers>().cloned().unwrap_or_default();
    let headers = headers_json(request.headers());
    let query = query_json(request.uri().query());
    let raw_available = decoded.raw_json_body(input).is_some();
    let body: JsonObject = serde_json::to_vec(&decoded.body)
        .and_then(|bytes| serde_json::from_slice(&bytes))
        .unwrap_or_default();
    let target = select_model(
        state,
        &rule,
        &mut markers,
        Selection {
            route: &input.route_path,
            body: &body,
            headers: &headers,
            query: &query,
            raw_available,
        },
    )
    .await
    .map_err(StageFailure::AuthContextSources)?;
    request.extensions_mut().insert(markers);
    Ok(cache_rewrite(request, input, decoded, target))
}

fn cache_rewrite(
    request: &mut Request<Body>,
    input: &mut RequestBodyInput,
    decoded: RequestBodyResult,
    target: Option<String>,
) -> Option<Vec<u8>> {
    input.cached_body = decoded.parsed_body;
    let Some(model) = target else {
        request.extensions_mut().insert(input.clone());
        return None;
    };
    let rewritten = rewrite_request_model(&decoded.body, Some(input), &model);
    input.cached_body = rewritten.cached_body;
    request.extensions_mut().insert(input.clone());
    rewritten.raw_json_body
}
