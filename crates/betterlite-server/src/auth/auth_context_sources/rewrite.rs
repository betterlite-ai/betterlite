use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, header};
use axum::response::Response;
use betterlite_auth::auth_context_sources::router_settings::TeamRouterSettings;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::RouteValue;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{RequestBodyInput, rewrite_request_model};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;

use crate::api::request_body::read_request_body;
use crate::auth::auth_identity::{AuthBackend, StageFailure, StageInput, dispatch};
use crate::auth::body_buffering::{BufferError, max_body_bytes, read_body};
use crate::auth::failures::{body_too_large_response, unreadable_body_response};
use crate::auth::{extract_key, headers_json, query_json};
use crate::state::ServerState;

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
    request: Request<Body>,
    master_key: &SecretString,
    failure_response: fn(&StageFailure) -> Response,
) -> Result<Request<Body>, Box<Response>> {
    if !is_llm_api_route(&RouteValue::Text(request.uri().path())) {
        return Ok(request);
    }
    // Authenticate before buffering; the existing stage loop still owns all subsequent authorization.
    let credential = StageInput::MasterKey(extract_key(request.headers()));
    if let Err(failure) = dispatch(AuthBackend::MasterKey, &credential, master_key) {
        return Err(Box::new(failure_response(&failure)));
    }
    let Some(state) = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(Arc::clone)
    else {
        return Ok(request);
    };
    buffered(request, &state, failure_response).await
}

async fn buffered(
    mut request: Request<Body>,
    state: &ServerState,
    failure_response: fn(&StageFailure) -> Response,
) -> Result<Request<Body>, Box<Response>> {
    let limit = max_body_bytes(&request);
    let stream = mem::replace(request.body_mut(), Body::empty());
    let bytes = read_body(stream, limit).await.map_err(|error| {
        Box::new(match error {
            BufferError::TooLarge { observed } => body_too_large_response(observed, limit),
            BufferError::Unreadable(_reason) => unreadable_body_response(),
        })
    })?;
    let mut input = RequestBodyInput::default();
    input.route_path = request.uri().path().to_owned();
    input.method = request.method().to_string();
    input.headers = request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.to_string(), text.to_owned()))
        })
        .collect();
    input.body = bytes.to_vec();
    let rewritten = rewrite(&mut request, state, &mut input)
        .await
        .map_err(|failure| Box::new(failure_response(&failure)))?;
    if rewritten.is_some() {
        request.headers_mut().remove(header::CONTENT_LENGTH);
    }
    *request.body_mut() = Body::from(rewritten.unwrap_or_else(|| bytes.to_vec()));
    Ok(request)
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
    let rule = request
        .extensions()
        .get::<Arc<RewriteRule>>()
        .map(Arc::clone)
        .unwrap_or_default();
    let mut markers = request
        .extensions()
        .get::<Markers>()
        .cloned()
        .unwrap_or_default();
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
    .map_err(|error| StageFailure::AuthContextSources(Box::new(error)))?;
    request.extensions_mut().insert(markers);
    input.cached_body = decoded.parsed_body;
    let Some(model) = target else {
        return Ok(None);
    };
    let rewritten = rewrite_request_model(&decoded.body, Some(input), &model);
    input.cached_body = rewritten.cached_body;
    request.extensions_mut().insert(input.clone());
    Ok(rewritten.raw_json_body)
}
