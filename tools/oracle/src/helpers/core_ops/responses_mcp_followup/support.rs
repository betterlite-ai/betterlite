use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::responses_mcp_events::event::{EventField, StreamEvent};
use betterlite_core::api::responses_mcp_followup::events::LiteLlmCallEvents;
use betterlite_core::api::responses_mcp_followup::records::{
    FollowUpCall, IdSource, MetadataAuth, ResponsesMcpFollowupInput, ResponsesMcpFollowupResult,
    StreamingAuth,
};
use betterlite_core::api::responses_mcp_followup::{FollowUpProvider, ResponsesMcpFollowup};
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{AuthSpec, RequestSpec};

pub(super) static BUILDER: LiteLlmCallEvents = LiteLlmCallEvents;

#[derive(Debug, Clone, Copy)]
pub(super) struct Settings {
    pub(super) should_auto_execute: bool,
    pub(super) original_stream: bool,
}

#[derive(Debug, Deserialize)]
struct AuthView {
    user_id: Option<String>,
}

pub(super) struct Provider {
    response: ResponsesApiResponse,
    failure: Option<u16>,
    calls: Mutex<Vec<FollowUpCall>>,
}

impl Provider {
    pub(super) fn answering(response: ResponsesApiResponse, failure: Option<u16>) -> Self {
        Self {
            response,
            failure,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Result<Vec<FollowUpCall>, Box<HelperError>> {
        let recorded = self
            .calls
            .lock()
            .map_err(|error| helper_error(&error.to_string()))?;
        Ok(recorded.to_vec())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("provider failure")]
pub(super) struct ProviderFailure {
    pub(super) status: u16,
}

#[async_trait::async_trait]
impl FollowUpProvider for Provider {
    type Error = ProviderFailure;

    async fn call(&self, call: &FollowUpCall) -> Result<ResponsesApiResponse, ProviderFailure> {
        self.calls
            .lock()
            .map_err(|_error| ProviderFailure { status: 0 })?
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(self.response.clone()),
            |status| Err(ProviderFailure { status }),
        )
    }
}

pub(super) fn service_input(
    request: &RequestSpec,
    call_params: &ParameterMap,
) -> ResponsesMcpFollowupInput {
    let mut input = empty_input();
    input.input = request.input.clone();
    input.model = request.model.clone();
    input.tools = request.tools.clone();
    input.previous_response_id = request.previous_response_id.clone();
    input.call_params = call_params.clone();
    input
}

pub(super) fn empty_input() -> ResponsesMcpFollowupInput {
    ResponsesMcpFollowupInput {
        input: ResponsesInput::Text(String::new()),
        model: String::new(),
        tools: None,
        previous_response_id: None,
        call_params: ParameterMap::new(),
        kwargs: ParameterMap::new(),
        follow_up_input: Vec::new(),
        follow_up_response_id: None,
        mcp_tools_with_litellm_proxy: Vec::new(),
        mcp_discovery_events: Vec::new(),
        tool_server_map: BTreeMap::new(),
        auth: StreamingAuth {
            direct: None,
            metadata: MetadataAuth::Absent,
        },
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        mcp_tools_fetched: Vec::new(),
    }
}

pub(super) fn run_service(
    input: &ResponsesMcpFollowupInput,
    provider: &Provider,
    ids: &[String],
    settings: Settings,
) -> Result<ResponsesMcpFollowupResult, Box<HelperError>> {
    let runtime = create_runtime()?;
    runtime
        .block_on(service(provider, ids, settings).run(input))
        .map_err(|error| helper_error(&error.to_string()))
}

pub(super) fn service<'deps>(
    provider: &'deps Provider,
    ids: &[String],
    settings: Settings,
) -> ResponsesMcpFollowup<'deps, Provider> {
    ResponsesMcpFollowup {
        provider,
        ids: ids_source(ids),
        event_builder: &BUILDER,
        should_auto_execute: settings.should_auto_execute,
        original_stream: settings.original_stream,
    }
}

pub(super) fn create_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error(&error.to_string()))
}

pub(super) fn streaming_auth(spec: &AuthSpec) -> Result<StreamingAuth, Box<HelperError>> {
    match spec {
        AuthSpec::None => Ok(StreamingAuth {
            direct: None,
            metadata: MetadataAuth::Absent,
        }),
        AuthSpec::Direct { user_id } => Ok(StreamingAuth {
            direct: Some(principal(user_id)?),
            metadata: MetadataAuth::Absent,
        }),
        AuthSpec::Metadata { user_id } => Ok(StreamingAuth {
            direct: None,
            metadata: MetadataAuth::Mapping(match user_id {
                Some(user_id) => Some(principal(user_id)?),
                None => None,
            }),
        }),
    }
}

fn principal(user_id: &str) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let fields: BTreeMap<String, JsonData> = serde_json::from_value(json!({ "user_id": user_id }))
        .map_err(|error| helper_error(&error.to_string()))?;
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .map_err(|error| helper_error(&error.to_string()))
}

pub(super) fn auth_user_id(auth: Option<&UserAPIKeyAuth>) -> Option<String> {
    let resolved = auth?;
    let value: Value = serde_json::to_value(resolved).ok()?;
    serde_json::from_value::<AuthView>(value).ok()?.user_id
}

pub(super) fn stream_events(raw: &[BTreeMap<String, Value>]) -> Vec<StreamEvent> {
    raw.iter()
        .map(|fields| StreamEvent {
            fields: fields
                .iter()
                .map(|(name, value)| (name.clone(), EventField::Json(value.clone())))
                .collect(),
        })
        .collect()
}

pub(super) fn ids_source(values: &[String]) -> IdSource {
    let remaining = Arc::new(Mutex::new(VecDeque::from(values.to_vec())));
    Arc::new(move || {
        remaining.lock().map_or_else(
            |_| String::new(),
            |mut ids| ids.pop_front().unwrap_or_default(),
        )
    })
}

pub(super) fn minimal_response() -> Result<ResponsesApiResponse, Box<HelperError>> {
    serde_json::from_value(json!({
        "id": "resp_follow_up",
        "created_at": 1,
        "output": [],
    }))
    .map_err(|error| helper_error(&error.to_string()))
}

pub(super) fn helper_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
