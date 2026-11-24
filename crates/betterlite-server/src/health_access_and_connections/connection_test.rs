use axum::Json as AxumJson;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Request, State};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_auth::request_ban_catalog::RequestBanCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use serde::{Deserialize, Serialize};

use crate::auth::auth_identity::AuthContext;
use crate::auth::body_buffering::{BufferError, max_body_bytes, read_body};
use crate::auth::failures::{body_too_large_response, unreadable_body_response};
use crate::health_access_and_connections::connection_error::{
    ConnectionError, model_call_rejection,
};
use crate::health_access_and_connections::connection_flow::{
    ModelLookup, lookup_deployment, merge_request_params, reject_banned_params, take_mode,
};
use crate::health_access_and_connections::connection_state::TestConnectionState;
use crate::health_access_and_connections::credential_scope::reject_os_environ_references;
use crate::health_access_and_connections::visibility::HealthCaller;
use crate::health_probe_settings::display::clean_endpoint_data;
use crate::health_probe_settings::model_info::CatalogModelInfo;
use crate::health_probe_settings::prepare::update_params_for_health_check;
use crate::team_access::caller::TeamCaller;
use crate::workflow_runs::router::decode_body;

// the body mode is a Literal in the endpoint signature, so any other string is the 422 FastAPI raises
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    Chat,
    Completion,
    Embedding,
    AudioSpeech,
    AudioTranscription,
    ImageGeneration,
    ImageEdit,
    VideoGeneration,
    Batch,
    Rerank,
    Realtime,
    Responses,
    Ocr,
}

impl ConnectionMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Completion => "completion",
            Self::Embedding => "embedding",
            Self::AudioSpeech => "audio_speech",
            Self::AudioTranscription => "audio_transcription",
            Self::ImageGeneration => "image_generation",
            Self::ImageEdit => "image_edit",
            Self::VideoGeneration => "video_generation",
            Self::Batch => "batch",
            Self::Rerank => "rerank",
            Self::Realtime => "realtime",
            Self::Responses => "responses",
            Self::Ocr => "ocr",
        }
    }
}

// test_model_connection's three body fields, each optional as fastapi.Body(None) declares them
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ConnectionTestRequest {
    pub mode: Option<ConnectionMode>,
    pub litellm_params: Option<JsonObject>,
    pub model_info: Option<JsonObject>,
}

#[derive(Debug, Serialize)]
pub struct TestConnectionResponse {
    pub status: &'static str,
    pub result: JsonObject,
}

pub fn router(state: TestConnectionState) -> Router {
    Router::new()
        .route("/health/test_connection", post(test_connection))
        .with_state(state)
}

async fn test_connection(
    State(state): State<TestConnectionState>,
    identity: Option<Extension<AuthContext>>,
    request: Request,
) -> Response {
    let Some(Extension(context)) = identity else {
        return ConnectionError::Unauthenticated.into_response();
    };
    let limit = max_body_bytes(&request);
    let body = match read_body(request.into_body(), limit).await {
        Ok(body) => body,
        Err(BufferError::TooLarge { observed }) => {
            return body_too_large_response(observed, limit);
        }
        Err(BufferError::Unreadable(_reason)) => return unreadable_body_response(),
    };
    match run_test(&state, &context, &body).await {
        Ok(response) => AxumJson(response).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn run_test(
    state: &TestConnectionState,
    context: &AuthContext,
    body: &Bytes,
) -> Result<TestConnectionResponse, ConnectionError> {
    let request: ConnectionTestRequest = decode_body(body)?;
    if state.authorizer.is_none() {
        return Err(ConnectionError::Database);
    }
    let caller = HealthCaller::from_context(context)?;
    let ConnectionTestRequest {
        mode,
        litellm_params,
        model_info,
    } = request;
    let request_params = checked_params(state, litellm_params, model_info.as_ref())?;
    probe_authorized(state, &request_params, model_info, &caller, mode).await
}

// the os.environ/ scan and B884's URL check on everything the request itself supplied
fn checked_params(
    state: &TestConnectionState,
    litellm_params: Option<JsonObject>,
    model_info: Option<&JsonObject>,
) -> Result<JsonObject, ConnectionError> {
    let request_params = litellm_params.unwrap_or_default();
    reject_request_fields(&request_params, model_info)?;
    reject_connection_fields(
        &request_params,
        model_info,
        state.allow_client_side_credentials,
    )?;
    Ok(request_params)
}

// the lookup and authorization on the final probe params, then one probe bounded by the health check timeout
async fn probe_authorized(
    state: &TestConnectionState,
    request_params: &JsonObject,
    model_info: Option<JsonObject>,
    caller: &HealthCaller,
    mode: Option<ConnectionMode>,
) -> Result<TestConnectionResponse, ConnectionError> {
    let ModelLookup {
        config_params,
        model_info: loaded_model_info,
    } = lookup_deployment(state.router.as_deref(), request_params, model_info.as_ref());
    let resolved = loaded_model_info.or(model_info);
    let mut prepared = prepare_params(state, &config_params, request_params, resolved.as_ref())?;
    let team_caller = caller.team_caller();
    authorize(
        state,
        resolved.as_ref(),
        &team_caller,
        caller.user_role.as_deref(),
    )
    .await?;
    let probe_mode = mode.map_or_else(
        || take_mode(&mut prepared),
        |named| Some(named.as_str().to_owned()),
    );
    let result = state.probe.probe(&prepared, probe_mode.as_deref()).await?;
    Ok(respond(prepared, &result))
}

// the request's own values must carry no os.environ/ reference; config values were resolved before they got here
fn reject_request_fields(
    request_params: &JsonObject,
    request_model_info: Option<&JsonObject>,
) -> Result<(), ConnectionError> {
    reject_os_environ_references(request_params)?;
    if let Some(info) = request_model_info.filter(|info| !info.is_empty()) {
        reject_os_environ_references(info)?;
    }
    Ok(())
}

// B884's check on the params, the model_info and the health_check_params map the probe settings read
fn reject_connection_fields(
    request_params: &JsonObject,
    request_model_info: Option<&JsonObject>,
    allow_client_side_credentials: bool,
) -> Result<(), ConnectionError> {
    reject_banned_params(request_params, allow_client_side_credentials)?;
    let Some(info) = request_model_info else {
        return Ok(());
    };
    reject_banned_params(info, allow_client_side_credentials)?;
    if let Some(Json::Object(health_params)) = info.get("health_check_params") {
        reject_banned_params(health_params, allow_client_side_credentials)?;
    }
    Ok(())
}

fn prepare_params(
    state: &TestConnectionState,
    config_params: &JsonObject,
    request_params: &JsonObject,
    resolved_model_info: Option<&JsonObject>,
) -> Result<JsonObject, ConnectionError> {
    let catalog = RequestBanCatalog::pinned();
    let merged = merge_request_params(
        config_params,
        request_params,
        state.allow_client_side_credentials,
        &catalog.banned,
    );
    let empty = JsonObject::new();
    let model_info = resolved_model_info.unwrap_or(&empty);
    let models = CatalogModelInfo {
        catalog: state.models.as_ref(),
    };
    let mut random = StdRng::try_from_rng(&mut SysRng)
        .map_err(|source| ConnectionError::Random(source.to_string()))?;
    Ok(update_params_for_health_check(
        model_info,
        &merged,
        &state.policy,
        &models,
        &mut random,
    )?)
}

// can_user_make_model_call on the final probe params: the resolved model_info's team decides which caller may run it
async fn authorize(
    state: &TestConnectionState,
    resolved_model_info: Option<&JsonObject>,
    caller: &TeamCaller,
    role: Option<&str>,
) -> Result<(), ConnectionError> {
    let team_id = team_id_of(resolved_model_info)?;
    let Some(authorizer) = state.authorizer.as_ref() else {
        return Err(ConnectionError::Database);
    };
    authorizer
        .authorize(caller, team_id)
        .await
        .map_err(|error| model_call_rejection(&error, role))
}

fn team_id_of(resolved_model_info: Option<&JsonObject>) -> Result<Option<&str>, ConnectionError> {
    match resolved_model_info.and_then(|info| info.get("team_id")) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(team)) => Ok(Some(team.as_str())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConnectionError::TeamIdType)
        }
    }
}

// {**litellm_params, **result} through _clean_endpoint_data(details=True), with the status read off the raw result
fn respond(prepared: JsonObject, result: &JsonObject) -> TestConnectionResponse {
    let status = if result.contains_key("error") {
        "error"
    } else {
        "success"
    };
    let mut combined = prepared;
    for (key, value) in result {
        combined.insert(key.clone(), value.clone());
    }
    TestConnectionResponse {
        status,
        result: clean_endpoint_data(&combined, true),
    }
}
