use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::PoisonError;

use axum::Router;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::routing::post;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use betterlite_secret_managers::secret_lookup::SecretLookup;

use crate::auth::auth_identity::AuthContext;
use crate::endpoints::passthrough_config_reads::db_read::PASS_THROUGH_FIELD;
use crate::endpoints::passthrough_config_reads::db_read::find_endpoint_by_id;
use crate::endpoints::passthrough_config_reads::record::EndpointRecordError;
use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use crate::passthrough::route_registration::PassThroughRoutes;
use crate::passthrough::route_registration::PassthroughParams;
use crate::passthrough::route_registration::RouteSpec;
use crate::passthrough::route_registration::SharedPassThroughRoutes;
use crate::passthrough::route_registration::add_exact_path_route;
use crate::passthrough::route_registration::add_subpath_route;
use crate::passthrough::route_registration::set_env_variables_in_header;
use crate::passthrough_route_registry::removal::remove_endpoint_routes;

use super::convert::guardrails_object;
use super::convert::json_object;
use super::create::create_endpoint;
use super::delete::delete_endpoint;
use super::error::LookupKind;
use super::error::PassthroughWriteError;
use super::input::EndpointInput;
use super::settings_port::ConfigFieldUpdate;
use super::settings_port::GeneralSettingsStore;
use super::update::update_endpoint;

pub struct PassThroughWriteState<Store> {
    pub settings: Arc<Store>,
    pub routes: SharedPassThroughRoutes,
    pub secrets: Arc<SecretLookup>,
    pub environment: Arc<BTreeMap<String, String>>,
}

// B8640 mounts these routes with B8641's settings store
pub fn routes<Store>(state: PassThroughWriteState<Store>) -> Router
where
    Store: GeneralSettingsStore + 'static,
{
    Router::new()
        .route(
            "/config/pass_through_endpoint",
            post(create_endpoint::<Store>).delete(delete_endpoint::<Store>),
        )
        .route(
            "/config/pass_through_endpoint/{endpoint_id}",
            post(update_endpoint::<Store>),
        )
        .with_state(Arc::new(state))
}

// FastAPI solves user_api_key_auth before it validates the body or the query
pub(super) fn require_caller(
    caller: Option<Extension<AuthContext>>,
) -> Result<(), PassthroughWriteError> {
    caller.map_or(Err(PassthroughWriteError::Unauthenticated), |_context| {
        Ok(())
    })
}

pub(super) fn decode_body(body: &[u8]) -> Result<EndpointInput, PassthroughWriteError> {
    if body.is_empty() {
        return Err(PassthroughWriteError::MissingBody);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let input = serde_path_to_error::deserialize(&mut deserializer).map_err(|source| {
        PassthroughWriteError::Body {
            source: Box::new(source),
        }
    })?;
    deserializer
        .end()
        .map_err(|source| PassthroughWriteError::Trailing { source })?;
    Ok(input)
}

// LiteLLM catches every failure of the field read in these routes; None is the value it then treats as absent
pub(super) async fn read_entries<Store>(
    settings: &Store,
) -> Result<Option<Vec<JsonData>>, PassthroughWriteError>
where
    Store: GeneralSettingsStore,
{
    let Some(info) = settings.get_field(PASS_THROUGH_FIELD).await.ok() else {
        return Ok(None);
    };
    match info.field_value {
        JsonData::Null => Ok(None),
        JsonData::Array(entries) => Ok(Some(entries)),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Object(_) => {
            Err(PassthroughWriteError::StoredNotAList)
        }
    }
}

pub(super) async fn store_entries<Store>(
    settings: &Store,
    entries: Vec<JsonData>,
) -> Result<(), PassthroughWriteError>
where
    Store: GeneralSettingsStore,
{
    settings
        .update_field(ConfigFieldUpdate {
            field_name: PASS_THROUGH_FIELD.to_owned(),
            field_value: JsonData::Array(entries),
        })
        .await
        .map_err(PassthroughWriteError::Settings)
}

pub(super) fn require_endpoint(
    entries: &[JsonData],
    endpoint_id: &str,
    kind: LookupKind,
) -> Result<(PassThroughGenericEndpoint, usize), PassthroughWriteError> {
    let stored = find_endpoint_by_id(entries, endpoint_id)
        .map_err(PassthroughWriteError::Record)?
        .ok_or_else(|| PassthroughWriteError::MissingEndpoint {
            endpoint_id: endpoint_id.to_owned(),
            kind,
        })?;
    let index = endpoint_index(entries, endpoint_id)
        .map_err(PassthroughWriteError::Record)?
        .ok_or_else(|| PassthroughWriteError::MissingIndex {
            endpoint_id: endpoint_id.to_owned(),
            kind,
        })?;
    Ok((stored, index))
}

// LiteLLM looks the id up twice, and the second loop parses every entry, so a malformed one fails it
fn endpoint_index(
    entries: &[JsonData],
    endpoint_id: &str,
) -> Result<Option<usize>, EndpointRecordError> {
    for (index, entry) in entries.iter().enumerate() {
        let endpoint = PassThroughGenericEndpoint::from_json(entry, false)?;
        if endpoint.id.as_deref() == Some(endpoint_id) {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

pub(super) fn resolve_headers<Store>(
    state: &PassThroughWriteState<Store>,
    endpoint: &PassThroughGenericEndpoint,
) -> Result<Option<JsonObject>, PassthroughWriteError> {
    let headers = json_object(&endpoint.headers);
    set_env_variables_in_header(
        state.secrets.as_ref(),
        state.environment.as_ref(),
        Some(&headers),
    )
    .map_err(PassthroughWriteError::Registration)
}

pub(super) fn install_endpoint<Store>(
    state: &PassThroughWriteState<Store>,
    path_id: &str,
    endpoint: &PassThroughGenericEndpoint,
    headers: Option<JsonObject>,
) {
    let methods = endpoint.methods.clone().unwrap_or_default();
    let params = PassthroughParams {
        target: endpoint.target.clone(),
        custom_headers: headers,
        forward_headers: None,
        merge_query_params: None,
        default_query_params: Some(json_object(&endpoint.default_query_params)),
        cost_per_request: Some(endpoint.cost_per_request),
        guardrails: guardrails_object(endpoint.guardrails.as_ref()),
        timeout: endpoint.timeout,
        config_file_path: None,
    };
    let endpoint_id = endpoint.id.clone().unwrap_or_else(|| path_id.to_owned());
    let spec = RouteSpec {
        endpoint_id: &endpoint_id,
        path: &endpoint.path,
        methods: &methods,
        auth: endpoint.auth.0,
        params: &params,
    };
    if endpoint.include_subpath.0 {
        add_subpath_route(&state.routes, &spec, None);
    } else {
        add_exact_path_route(&state.routes, &spec, None);
    }
}

pub(super) fn remove_routes(routes: &SharedPassThroughRoutes, endpoint_id: &str) {
    let mut state = routes.lock().unwrap_or_else(PoisonError::into_inner);
    let PassThroughRoutes {
        registry,
        openai_routes,
        ..
    } = &mut *state;
    remove_endpoint_routes(&mut registry.registrations, openai_routes, endpoint_id);
}
