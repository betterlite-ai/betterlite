use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::{Extension, MatchedPath, Path, Request, State};
use axum::http::Method;
use axum::http::request::Parts;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::agents::json::AgentJson as JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use crate::auth::auth_identity::AuthContext;
use crate::auth::body_buffering::{BufferError, read_body};
use crate::auth::failures::{body_too_large_response, unreadable_body_response};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::route_type::RouteType;
use crate::pre_call_step::{PreCallInput, PreCallOutput};

use super::processing::AgentCall;
use super::{AGENT_VERSIONS_PATH, AgentCollectionState, body, guards, merge};

// which LiteLLM handler a request reached; the fixed provider and the path name are the only per-route data
enum AgentRoute {
    Create,
    List,
    Get(String),
    Delete(String),
    ListVersions(String),
}

impl AgentRoute {
    // each of the five LiteLLM handlers reports its own route_type to base_process_llm_request
    const fn route_type(&self) -> RouteType {
        match self {
            Self::Create => RouteType::CreateAgent,
            Self::List => RouteType::ListAgents,
            Self::Get(_) => RouteType::GetAgent,
            Self::Delete(_) => RouteType::DeleteAgent,
            Self::ListVersions(_) => RouteType::ListAgentVersions,
        }
    }
}

// the five agent routes share one handler; the path name, the matched pattern and the method pick the route type
pub(super) async fn agent<K>(
    State(state): State<AgentCollectionState<K>>,
    Extension(context): Extension<AuthContext>,
    Extension(key): Extension<K>,
    name: Option<Path<String>>,
    request: Request,
) -> Response
where
    K: PreCallKey + Clone + Send + Sync + 'static,
{
    let versions = request
        .extensions()
        .get::<MatchedPath>()
        .is_some_and(|matched| matched.as_str() == AGENT_VERSIONS_PATH);
    let route = match (name, versions) {
        (Some(Path(segment)), true) => AgentRoute::ListVersions(segment),
        (Some(Path(segment)), false) if request.method() == Method::DELETE => {
            AgentRoute::Delete(segment)
        }
        (Some(Path(segment)), false) => AgentRoute::Get(segment),
        (None, _) if request.method() == Method::POST => AgentRoute::Create,
        (None, _) => AgentRoute::List,
    };
    serve(&state, &context, &key, &route, request).await
}

// the shared tail of every route: read the body the create route carries, build the data, then guard and process
async fn serve<K>(
    state: &AgentCollectionState<K>,
    context: &AuthContext,
    key: &K,
    route: &AgentRoute,
    request: Request,
) -> Response
where
    K: PreCallKey,
{
    let (parts, request_body) = request.into_parts();
    let query_params = merge::query_values(parts.uri.query());
    let data = if matches!(route, AgentRoute::Create) {
        match create_data(request_body, &parts, &query_params, state).await {
            Ok(data) => data,
            Err(response) => return response,
        }
    } else {
        route_data(route, &query_params)
    };
    let call = Guarded {
        state,
        context,
        key,
        parts: &parts,
        route_type: route.route_type(),
    };
    guarded(call, data).await
}

// create reads the body before the guard, through _read_request_body's content-type rules
async fn create_data<K>(
    request_body: Body,
    parts: &Parts,
    query_params: &BTreeMap<String, String>,
    state: &AgentCollectionState<K>,
) -> Result<JsonMap, Response>
where
    K: PreCallKey,
{
    let limit = state.limits.max_json_bytes;
    let bytes = match read_body(request_body, limit).await {
        Ok(bytes) => bytes,
        Err(BufferError::TooLarge { observed }) => {
            return Err(body_too_large_response(observed, limit));
        }
        Err(BufferError::Unreadable(_reason)) => return Err(unreadable_body_response()),
    };
    let mut data = match body::read_json_body(
        parts,
        query_params,
        bytes.as_ref(),
        &state.request_body,
        &state.limits,
    ) {
        Ok(data) => data,
        Err(error) => return Err(body::failure(&error)),
    };
    merge::merge_body_template(&mut data);
    merge::default_provider(&mut data);
    Ok(data)
}

// the list and item routes start from the fixed provider and the path name, then merge the query template
fn route_data(route: &AgentRoute, query_params: &BTreeMap<String, String>) -> JsonMap {
    let mut data = JsonMap::new();
    if let AgentRoute::Get(name) | AgentRoute::Delete(name) | AgentRoute::ListVersions(name) = route
    {
        data.insert(String::from("name"), JsonValue::String(name.clone()));
    }
    data.insert(
        String::from(merge::PROVIDER_FIELD),
        JsonValue::String(String::from(merge::PROVIDER_DEFAULT)),
    );
    merge::merge_query_params_into_data(&mut data, query_params);
    data
}

// everything the guarded part of a route reads once its data is built
struct Guarded<'call, K: PreCallKey> {
    state: &'call AgentCollectionState<K>,
    context: &'call AuthContext,
    key: &'call K,
    parts: &'call Parts,
    route_type: RouteType,
}

// the guard runs before the pre-call and the call, so its 401 never enters the exception mapping
async fn guarded<K>(call: Guarded<'_, K>, data: JsonMap) -> Response
where
    K: PreCallKey,
{
    let Guarded {
        state,
        context,
        key,
        parts,
        route_type,
    } = call;
    if let Err(missing) = guards::enforce_provider_key(&data, &context.principal) {
        return missing.into_response();
    }
    let input = PreCallInput {
        data,
        request: parts,
        key,
        route_type,
        version: Some(state.version.as_str()),
        model: None,
        skip_guardrails: false,
    };
    let output = match state.pre_call.pre_call(input).await {
        Ok(output) => output,
        Err(error) => {
            return state
                .processing
                .handle_exception(error.failure, &error.data, key)
                .await;
        }
    };
    processed(state, key, parts, route_type, output).await
}

async fn processed<K>(
    state: &AgentCollectionState<K>,
    key: &K,
    parts: &Parts,
    route_type: RouteType,
    output: PreCallOutput,
) -> Response
where
    K: PreCallKey,
{
    let PreCallOutput { data, logging } = output;
    let call = AgentCall {
        data: &data,
        logging,
        request: parts,
        key,
        route_type,
        version: state.version.as_str(),
    };
    match state.processing.process(call).await {
        Ok(response) => response,
        Err(failure) => state.processing.handle_exception(failure, &data, key).await,
    }
}
