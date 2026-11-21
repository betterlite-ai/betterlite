use betterlite_auth::auth_request_models::sources::{DispatchContext, is_pass_through_dispatch};
use betterlite_context::owned_parameter_names::{Owner, owned_names, wire_names};
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::{
    UserKeyLogMetadata, logged_api_key,
};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use indexmap::map::Entry;
use thiserror::Error;

use super::PassthroughLogPayload;
use super::metadata_tags::update_metadata_tags;
use crate::failure_hook::lift::PASS_THROUGH_CALL_TYPE;

pub const SESSION_ID_OMITTED_METADATA_KEY: &str = "litellm_session_id_omitted"; // ast-grep-ignore: no-long-names

pub const MODEL_ACCESS_GROUP_METADATA_KEY: &str = "user_api_key_matched_model_access_groups"; // ast-grep-ignore: no-long-names
const KEPT_OUT_METADATA_KEYS: [&str; 2] = ["litellm_metadata", "metadata"];
const MODEL_MAX_BUDGET_KEY: &str = "user_api_key_model_max_budget";
const TEAM_MODEL_MAX_BUDGET_KEY: &str = "user_api_key_team_model_max_budget";
const USER_MODEL_MAX_BUDGET_KEY: &str = "user_api_key_user_model_max_budget";
const END_USER_MODEL_MAX_BUDGET_KEY: &str = "user_api_key_end_user_model_max_budget";
const USER_API_KEY_KEY: &str = "user_api_key";
const PARENT_SPAN_KEY: &str = "litellm_parent_otel_span";
const BUDGET_RESERVATION_KEY: &str = "user_api_key_budget_reservation";
const MODEL_INFO_KEY: &str = "model_info";

// the request fields _init_kwargs_for_pass_through_endpoint reads
pub struct PassThroughRequest<'request> {
    pub url: &'request str,
    pub method: &'request str,
    pub headers: &'request HeaderMap,
    pub deployment_model_info: Option<&'request Metadata>,
}

// UserAPIKeyAuth's pass-through slice: B1106's principal facts plus the span and per-model budgets it stamps
pub struct PassThroughKey {
    pub principal: PrincipalFacts,
    pub parent_otel_span: Json,
    pub model_max_budget: Option<Metadata>,
    pub team_model_max_budget: Option<Metadata>,
    pub user_model_max_budget: Option<Metadata>,
    pub end_user_model_max_budget: Option<Metadata>,
}

impl Default for PassThroughKey {
    fn default() -> Self {
        Self {
            principal: PrincipalFacts::default(),
            parent_otel_span: Json::Null,
            model_max_budget: None,
            team_model_max_budget: None,
            user_model_max_budget: None,
            end_user_model_max_budget: None,
        }
    }
}

// the LiteLLMLoggingObj state these helpers read and write
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassthroughLoggingObj {
    pub model_call_details: Metadata,
}

impl PassthroughLoggingObj {
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut details = Metadata::new();
        details.insert(
            "model_call_details".to_owned(),
            Json::Object(self.model_call_details.clone()),
        );
        Json::Object(details)
    }
}

pub struct PassThroughInit<'request> {
    pub request: PassThroughRequest<'request>,
    pub user_api_key_dict: &'request PassThroughKey,
    pub passthrough_logging_payload: PassthroughLogPayload,
    pub logging_obj: &'request mut PassthroughLoggingObj,
    pub parsed_body: Option<RequestData>,
    pub litellm_call_id: Option<String>,
    pub dispatch: &'request DispatchContext<'request>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassThroughKwargs {
    pub litellm_params: RequestData,
    pub call_type: &'static str,
    pub litellm_call_id: Option<String>,
    pub passthrough_logging_payload: PassthroughLogPayload,
}

impl PassThroughKwargs {
    #[must_use]
    pub fn to_data(&self) -> RequestData {
        let mut data = RequestData::new();
        data.insert(
            "litellm_params".to_owned(),
            Json::Object(self.litellm_params.clone()),
        );
        data.insert(
            "call_type".to_owned(),
            Json::Text(self.call_type.to_owned()),
        );
        data.insert(
            "litellm_call_id".to_owned(),
            self.litellm_call_id
                .as_ref()
                .map_or(Json::Null, |call_id| Json::Text(call_id.clone())),
        );
        data.insert(
            "passthrough_logging_payload".to_owned(),
            self.passthrough_logging_payload.to_json(),
        );
        data
    }
}

#[derive(Debug, Error)]
pub enum PassThroughContextError {
    #[error("the {field} field of the request body is not a JSON object")]
    MetadataNotObject { field: &'static str },
    #[error("the authenticated key metadata could not be encoded")]
    KeyMetadata {
        #[source]
        source: serde_json::Error,
    },
}

// _init_kwargs_for_pass_through_endpoint: the kwargs a pass-through call logs and forwards with
pub fn init_pass_through_kwargs(
    input: PassThroughInit<'_>,
) -> Result<PassThroughKwargs, PassThroughContextError> {
    let mut parsed_body = input.parsed_body.unwrap_or_default();
    let litellm_keys_in_body = take_litellm_keys(&mut parsed_body);
    let mut metadata = sanitized_key_metadata(&input.user_api_key_dict.principal)?;
    metadata.extend(client_metadata(&litellm_keys_in_body)?);
    update_metadata_tags(input.request.headers, &mut metadata);
    stamp_internal_keys(&mut metadata, input.user_api_key_dict, input.dispatch);
    // the authenticated facts win over a client body that mirrors their names
    metadata.extend(sanitized_key_metadata(&input.user_api_key_dict.principal)?);
    if let Some(model_info) = input.request.deployment_model_info {
        metadata.insert(MODEL_INFO_KEY.to_owned(), Json::Object(model_info.clone()));
    }
    let litellm_params = without_kept_out(litellm_keys_in_body);
    let payload = input.passthrough_logging_payload;
    input
        .logging_obj
        .model_call_details
        .insert("passthrough_logging_payload".to_owned(), payload.to_json());
    Ok(PassThroughKwargs {
        litellm_params: pass_through_litellm_params(
            litellm_params,
            metadata,
            &input.request,
            parsed_body,
        ),
        call_type: PASS_THROUGH_CALL_TYPE,
        litellm_call_id: input.litellm_call_id,
        passthrough_logging_payload: payload,
    })
}

fn take_litellm_keys(parsed_body: &mut RequestData) -> RequestData {
    let mut taken = RequestData::new();
    for name in owned_names() {
        if let Some(value) = parsed_body.shift_remove(&name) {
            taken.insert(name, value);
        }
    }
    taken
}

fn kept_out_names() -> Vec<String> {
    let mut names: Vec<String> = KEPT_OUT_METADATA_KEYS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    names.extend(wire_names(Owner::ProxyRequestState));
    names
}

fn without_kept_out(litellm_keys: RequestData) -> RequestData {
    let kept_out = kept_out_names();
    litellm_keys
        .into_iter()
        .filter(|(name, _value)| !kept_out.contains(name))
        .collect()
}

fn client_metadata(body: &RequestData) -> Result<Metadata, PassThroughContextError> {
    let mut merged = Metadata::new();
    for field in KEPT_OUT_METADATA_KEYS {
        match body.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Object(values)) => merged.extend(values.clone()),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(PassThroughContextError::MetadataNotObject { field });
            }
        }
    }
    Ok(merged)
}

fn sanitized_key_metadata(principal: &PrincipalFacts) -> Result<Metadata, PassThroughContextError> {
    let logged = UserKeyLogMetadata::from(principal);
    let encoded = serde_json::to_string(&logged)
        .map_err(|source| PassThroughContextError::KeyMetadata { source })?;
    serde_json::from_str(&encoded).map_err(|source| PassThroughContextError::KeyMetadata { source })
}

// the authenticated facts land after the client merge so a request body cannot clobber them
fn stamp_internal_keys(
    metadata: &mut Metadata,
    user_api_key_dict: &PassThroughKey,
    dispatch: &DispatchContext<'_>,
) {
    stamp_identity_keys(metadata, user_api_key_dict);
    if !is_pass_through_dispatch(dispatch) {
        stamp_budget_keys(metadata, user_api_key_dict);
    }
}

fn stamp_identity_keys(metadata: &mut Metadata, user_api_key_dict: &PassThroughKey) {
    let principal = &user_api_key_dict.principal;
    metadata.shift_remove(SESSION_ID_OMITTED_METADATA_KEY); // ast-grep-ignore: no-long-names
    metadata.insert(
        USER_API_KEY_KEY.to_owned(),
        logged_api_key(principal).map_or(Json::Null, |key| Json::Text(key.to_owned())),
    );
    metadata.insert(
        PARENT_SPAN_KEY.to_owned(),
        user_api_key_dict.parent_otel_span.clone(),
    );
    metadata.insert(
        BUDGET_RESERVATION_KEY.to_owned(),
        principal
            .budget_reservation
            .as_ref()
            .map_or(Json::Null, |object| Json::Object(object.clone())),
    );
    metadata.insert(
        MODEL_ACCESS_GROUP_METADATA_KEY.to_owned(), // ast-grep-ignore: no-long-names
        Json::List(
            principal
                .matched_model_access_groups
                .iter()
                .cloned()
                .map(Json::Text)
                .collect(),
        ),
    );
}

// only the built-in provider routes charge the per-model counters a user-defined route cannot reach
fn stamp_budget_keys(metadata: &mut Metadata, user_api_key_dict: &PassThroughKey) {
    for (name, budget) in [
        (MODEL_MAX_BUDGET_KEY, &user_api_key_dict.model_max_budget),
        (
            TEAM_MODEL_MAX_BUDGET_KEY,
            &user_api_key_dict.team_model_max_budget,
        ),
        (
            USER_MODEL_MAX_BUDGET_KEY,
            &user_api_key_dict.user_model_max_budget,
        ),
        (
            END_USER_MODEL_MAX_BUDGET_KEY,
            &user_api_key_dict.end_user_model_max_budget,
        ),
    ] {
        metadata.insert(
            name.to_owned(),
            budget
                .as_ref()
                .map_or(Json::Null, |object| Json::Object(object.clone())),
        );
    }
}

fn pass_through_litellm_params(
    mut params: RequestData,
    metadata: Metadata,
    request: &PassThroughRequest<'_>,
    parsed_body: RequestData,
) -> RequestData {
    let mut proxy_server_request = IndexMap::new();
    proxy_server_request.insert("url".to_owned(), Json::Text(request.url.to_owned()));
    proxy_server_request.insert("method".to_owned(), Json::Text(request.method.to_owned()));
    proxy_server_request.insert("body".to_owned(), Json::Object(parsed_body));
    proxy_server_request.insert("headers".to_owned(), request_headers_json(request.headers));
    params.insert("metadata".to_owned(), Json::Object(metadata));
    params.insert(
        "proxy_server_request".to_owned(),
        Json::Object(proxy_server_request),
    );
    params
}

// dict(request.headers): httpx joins a repeated header's values with ", "
fn request_headers_json(headers: &HeaderMap) -> Json {
    let mut object = IndexMap::new();
    for (name, value) in headers {
        let Ok(text) = value.to_str() else {
            continue;
        };
        match object.entry(name.as_str().to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(Json::Text(text.to_owned()));
            }
            Entry::Occupied(mut entry) => {
                if let Json::Text(joined) = entry.get_mut() {
                    joined.push_str(", ");
                    joined.push_str(text);
                }
            }
        }
    }
    Json::Object(object)
}

pub(super) const fn as_object(value: &Json) -> Option<&Metadata> {
    if let Json::Object(fields) = value {
        return Some(fields);
    }
    None
}
