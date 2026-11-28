use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::mock_testing_params::Settings as MockTestingParams;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};
use betterlite_openai::openai_base_config::Object as JsonMap;

use crate::route_read_through::{ModelNotFoundCause, ModelReadThrough, route_with_read_through};

pub mod outcome;
pub mod plan;
pub mod standalone;
pub mod steps;

mod tests;

use outcome::{MessagesDispatch, MessagesOutcome, MessagesRouting};
use plan::{MessagesRoute, MessagesRouteInput, RouteFlag, plan_messages_route, team_id_from_data};
use steps::{CallStepError, mock_params_disallowed, required_param_missing};

// The settings the router accepts as per-request kwargs, in LiteLLM's order
const OVERRIDE_KEYS: [&str; 8] = [
    "fallbacks",
    "context_window_fallbacks",
    "content_policy_fallbacks",
    "num_retries",
    "timeout",
    "model_group_retry_policy",
    "routing_strategy",
    "enable_tag_filtering",
];

// The ports and settings one route's call steps take; the route type travels beside them so every error names the route
pub struct MessagesCallParts {
    pub dispatch: Arc<dyn MessagesDispatch>,
    pub routing: Option<Arc<dyn MessagesRouting>>,
    pub read_through: Option<Arc<dyn ModelReadThrough>>,
    pub mock: MockTestingParams,
    pub user_model: Option<String>,
}

pub struct MessagesCallSteps {
    route_type: &'static str,
    dispatch: Arc<dyn MessagesDispatch>,
    routing: Option<Arc<dyn MessagesRouting>>,
    read_through: Option<Arc<dyn ModelReadThrough>>,
    mock: MockTestingParams,
    user_model: Option<String>,
}

impl MessagesCallSteps {
    #[must_use]
    pub fn new(route_type: &'static str, parts: MessagesCallParts) -> Self {
        Self {
            route_type,
            dispatch: parts.dispatch,
            routing: parts.routing,
            read_through: parts.read_through,
            mock: parts.mock,
            user_model: parts.user_model,
        }
    }

    // route_request: one attempt, then one more after a read-through that answers true
    pub async fn call(
        &self,
        data: JsonMap,
        is_proxy_admin: bool,
    ) -> Result<MessagesOutcome, MessagesCallError> {
        let model = data
            .get("model")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let source = Arc::new(data);
        let attempt = async || self.attempt(&source, is_proxy_admin).await;
        route_with_read_through(model.as_deref(), self.read_through.as_deref(), attempt).await
    }

    // Every attempt starts from the data as it was before the first one
    async fn attempt(
        &self,
        source: &JsonMap,
        is_proxy_admin: bool,
    ) -> Result<MessagesOutcome, MessagesCallError> {
        let mut data = source.clone();
        required_param_missing(self.route_type, &data)?;
        mock_params_disallowed(&data, self.mock.allowed)?;
        data.remove("enable_tag_filtering");
        let input = route_input(&data, is_proxy_admin, self.user_model.as_deref());
        let plan = plan_messages_route(&input, self.routing.as_deref());
        if input.has_override.0 && !input.has_api_key_or_base.0 && !input.has_user_config.0 {
            merge_overrides(&mut data);
        }
        self.run_plan(data, &input, plan).await
    }

    async fn run_plan(
        &self,
        data: JsonMap,
        input: &MessagesRouteInput,
        plan: MessagesRoute,
    ) -> Result<MessagesOutcome, MessagesCallError> {
        match plan {
            MessagesRoute::Router {
                specific_deployment,
            } => {
                let routing = self.routing.as_deref().ok_or_else(routing_missing)?;
                call_router(routing, data, input, specific_deployment).await
            }
            MessagesRoute::Direct => self.dispatch.dispatch(data).await,
            MessagesRoute::A2aAgent => self.call_a2a(data, input).await,
            MessagesRoute::UserConfig => Err(MessagesCallError::UserConfig),
            MessagesRoute::Blocked => Err(MessagesCallError::Blocked),
            MessagesRoute::MissingModel => Err(MessagesCallError::MissingModel),
            MessagesRoute::NotFound => {
                Err(MessagesCallError::NotFound(ProxyModelNotFoundError::new(
                    route_endpoint(self.route_type).unwrap_or(self.route_type),
                    input.model.as_deref().unwrap_or_default(),
                    true,
                )))
            }
        }
    }

    async fn call_a2a(
        &self,
        data: JsonMap,
        input: &MessagesRouteInput,
    ) -> Result<MessagesOutcome, MessagesCallError> {
        let Some(routing) = self.routing.as_deref() else {
            return Err(routing_missing());
        };
        let Some(result) = routing.a2a_agent(data).await else {
            return Err(MessagesCallError::NotFound(ProxyModelNotFoundError::new(
                route_endpoint(self.route_type).unwrap_or(self.route_type),
                input.model.as_deref().unwrap_or_default(),
                true,
            )));
        };
        result
    }
}

async fn call_router(
    routing: &dyn MessagesRouting,
    mut data: JsonMap,
    input: &MessagesRouteInput,
    specific_deployment: bool,
) -> Result<MessagesOutcome, MessagesCallError> {
    // The team mapping runs only where LiteLLM reaches the router, never on the api_key or override branches
    if !input.has_api_key_or_base.0
        && !input.has_override.0
        && let Some(team_id) = input.team_id.as_deref()
        && let Some(mapped) = routing.map_team_model(input.model.as_deref(), team_id)
    {
        data.insert("model".to_owned(), JsonValue::String(mapped));
    }
    routing.anthropic_messages(data, specific_deployment).await
}

fn route_input(
    data: &JsonMap,
    is_proxy_admin: bool,
    user_model: Option<&str>,
) -> MessagesRouteInput {
    MessagesRouteInput {
        model: data
            .get("model")
            .and_then(JsonValue::as_str)
            .map(str::to_owned),
        has_model: data.contains_key("model"),
        team_id: team_id_from_data(data),
        is_proxy_admin,
        has_api_key_or_base: RouteFlag(
            data.contains_key("api_key") || data.contains_key("api_base"),
        ),
        has_user_config: RouteFlag(data.contains_key("user_config")),
        has_override: RouteFlag(data.contains_key("router_settings_override")),
        user_model: user_model.map(str::to_owned),
    }
}

// The override branch pops the setting and keeps only the keys the body does not carry itself
fn merge_overrides(data: &mut JsonMap) {
    let Some(JsonValue::Object(settings)) = data.remove("router_settings_override") else {
        return;
    };
    for key in OVERRIDE_KEYS {
        if let Some(value) = settings.get(key)
            && !data.contains_key(key)
        {
            data.insert(key.to_owned(), value.clone());
        }
    }
}

fn routing_missing() -> MessagesCallError {
    MessagesCallError::Call(Box::new(CallFailure {
        status: None,
        message: "the route chose the router, but the request carries no routing port".to_owned(),
        error_type: None,
        param: None,
        headers: BTreeMap::new(),
        timeout: None,
        cooldown: None,
    }))
}

// What B081's generic error answer reads off LiteLLM's exception
#[derive(Debug, Clone, PartialEq)]
pub struct CallFailure {
    pub status: Option<u16>,
    pub message: String,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub timeout: Option<f64>,
    pub cooldown: Option<f64>,
}

// LiteLLM's AnthropicContextManagementError, which /v1/messages answers in the Anthropic shape
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Context {
    pub status: u16,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MessagesCallError {
    #[error(transparent)]
    Steps(#[from] CallStepError),
    // boxed to keep the error under result_large_err's 128-byte limit
    #[error("{}", .0.message)]
    Call(Box<CallFailure>),
    #[error(transparent)]
    Context(#[from] Context),
    #[error("litellm.PermissionDeniedError: Model is blocked")]
    Blocked,
    #[error("model is a required field")]
    MissingModel,
    #[error(transparent)]
    NotFound(#[from] ProxyModelNotFoundError),
    #[error("user_config is not supported on /v1/messages")]
    UserConfig,
}

impl MessagesCallError {
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::Steps(step) => match step {
                CallStepError::MissingParam { .. } | CallStepError::MockParamsDisabled { .. } => {
                    400
                }
            },
            Self::Call(failure) => failure.status.unwrap_or(500),
            Self::Context(context) => context.status,
            Self::Blocked => 403,
            Self::MissingModel | Self::NotFound(_) | Self::UserConfig => 400,
        }
    }

    // The two plan outcomes LiteLLM answers with an HTTPException carry {"error": ...}
    #[must_use]
    pub fn http_detail(&self) -> Option<JsonValue> {
        match self {
            Self::Steps(CallStepError::MockParamsDisabled { .. }) => {
                Some(error_body(self.to_string().as_str()))
            }
            Self::NotFound(not_found) => Some(error_body(not_found.detail.error.as_str())),
            Self::Steps(CallStepError::MissingParam { .. })
            | Self::Call(_)
            | Self::Context(_)
            | Self::Blocked
            | Self::MissingModel
            | Self::UserConfig => None,
        }
    }
}

fn error_body(message: &str) -> JsonValue {
    let mut detail = JsonMap::new();
    detail.insert("error".to_owned(), JsonValue::String(message.to_owned()));
    JsonValue::Object(detail)
}

impl ModelNotFoundCause for MessagesCallError {
    fn model_not_found(&self) -> Option<&ProxyModelNotFoundError> {
        match self {
            Self::NotFound(error) => Some(error),
            Self::Steps(_)
            | Self::Call(_)
            | Self::Context(_)
            | Self::Blocked
            | Self::MissingModel
            | Self::UserConfig => None,
        }
    }
}
