use std::collections::BTreeMap;

use axum::extract::{Extension, Path, Request, State};
use axum::http::request::Parts;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::agents::json::AgentJson as JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use crate::auth::auth_identity::AuthContext;
use crate::auth::body_buffering::{BufferError, read_body};
use crate::auth::failures::{body_too_large_response, unreadable_body_response};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::route_type::RouteType;
use crate::pre_call_step::PreCall as _;
use crate::pre_call_step::{PreCallInput, PreCallOutput};

use super::processing::{AgentCall, AgentRequestProcessing as _};
use super::{AgentCollectionState, body, guards, merge};

// everything the guarded part of a route reads once its data is built
struct Guarded<'call, K> {
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

fn item_data(name: &str, query_params: &BTreeMap<String, String>) -> JsonMap {
    let mut data = JsonMap::new();
    data.insert(String::from("name"), JsonValue::String(name.to_owned()));
    data.insert(
        String::from(merge::PROVIDER_FIELD),
        JsonValue::String(String::from(merge::PROVIDER_DEFAULT)),
    );
    merge::merge_query_params_into_data(&mut data, query_params);
    data
}

fn list_data(query_params: &BTreeMap<String, String>) -> JsonMap {
    let mut data = JsonMap::new();
    data.insert(
        String::from(merge::PROVIDER_FIELD),
        JsonValue::String(String::from(merge::PROVIDER_DEFAULT)),
    );
    merge::merge_query_params_into_data(&mut data, query_params);
    data
}

// create_gemini_agent: read the body, pop and merge its template with body precedence, then default the provider
pub(super) async fn create_agent<K>(
    State(state): State<AgentCollectionState<K>>,
    Extension(context): Extension<AuthContext>,
    Extension(key): Extension<K>,
    request: Request,
) -> Response
where
    K: PreCallKey + Clone + Send + Sync + 'static,
{
    let (parts, request_body) = request.into_parts();
    let query_params = merge::query_values(parts.uri.query());
    let limit = state.limits.max_json_bytes;
    let bytes = match read_body(request_body, limit).await {
        Ok(bytes) => bytes,
        Err(BufferError::TooLarge { observed }) => return body_too_large_response(observed, limit),
        Err(BufferError::Unreadable(_reason)) => return unreadable_body_response(),
    };
    let mut data = match body::read_json_body(
        &parts,
        &query_params,
        bytes.as_ref(),
        &state.request_body,
        &state.limits,
    ) {
        Ok(data) => data,
        Err(error) => return body::failure(&error),
    };
    merge::merge_body_template(&mut data);
    merge::default_provider(&mut data);
    let call = Guarded {
        state: &state,
        context: &context,
        key: &key,
        parts: &parts,
        route_type: RouteType::CreateAgent,
    };
    guarded(call, data).await
}

// list_gemini_agents: the fixed provider is in data before the query template merges, so the template cannot replace it
pub(super) async fn list_agents<K>(
    State(state): State<AgentCollectionState<K>>,
    Extension(context): Extension<AuthContext>,
    Extension(key): Extension<K>,
    request: Request,
) -> Response
where
    K: PreCallKey + Clone + Send + Sync + 'static,
{
    let (parts, _request_body) = request.into_parts();
    let query_params = merge::query_values(parts.uri.query());
    let call = Guarded {
        state: &state,
        context: &context,
        key: &key,
        parts: &parts,
        route_type: RouteType::ListAgents,
    };
    guarded(call, list_data(&query_params)).await
}

pub(super) async fn get_agent<K>(
    State(state): State<AgentCollectionState<K>>,
    Extension(context): Extension<AuthContext>,
    Extension(key): Extension<K>,
    Path(name): Path<String>,
    request: Request,
) -> Response
where
    K: PreCallKey + Clone + Send + Sync + 'static,
{
    let (parts, _request_body) = request.into_parts();
    let query_params = merge::query_values(parts.uri.query());
    let call = Guarded {
        state: &state,
        context: &context,
        key: &key,
        parts: &parts,
        route_type: RouteType::GetAgent,
    };
    guarded(call, item_data(&name, &query_params)).await
}

pub(super) async fn delete_agent<K>(
    State(state): State<AgentCollectionState<K>>,
    Extension(context): Extension<AuthContext>,
    Extension(key): Extension<K>,
    Path(name): Path<String>,
    request: Request,
) -> Response
where
    K: PreCallKey + Clone + Send + Sync + 'static,
{
    let (parts, _request_body) = request.into_parts();
    let query_params = merge::query_values(parts.uri.query());
    let call = Guarded {
        state: &state,
        context: &context,
        key: &key,
        parts: &parts,
        route_type: RouteType::DeleteAgent,
    };
    guarded(call, item_data(&name, &query_params)).await
}

pub(super) async fn list_agent_versions<K>(
    State(state): State<AgentCollectionState<K>>,
    Extension(context): Extension<AuthContext>,
    Extension(key): Extension<K>,
    Path(name): Path<String>,
    request: Request,
) -> Response
where
    K: PreCallKey + Clone + Send + Sync + 'static,
{
    let (parts, _request_body) = request.into_parts();
    let query_params = merge::query_values(parts.uri.query());
    let call = Guarded {
        state: &state,
        context: &context,
        key: &key,
        parts: &parts,
        route_type: RouteType::ListAgentVersions,
    };
    guarded(call, item_data(&name, &query_params)).await
}
