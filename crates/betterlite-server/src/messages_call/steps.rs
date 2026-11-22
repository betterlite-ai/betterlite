use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_model_not_found::route_endpoint;
use betterlite_openai::openai_base_config::Object as JsonMap;

// LiteLLM's GATED_MOCK_PARAM_NAMES in its order, which is not the request's key order
pub const GATED_MOCK_PARAM_NAMES: [&str; 6] = [
    "mock_testing_fallbacks",
    "mock_testing_context_fallbacks",
    "mock_testing_content_policy_fallbacks",
    "mock_testing_rate_limit_error",
    "mock_timeout",
    "mock_delay",
];

pub const MOCK_TESTING_CONFIG_KEY: &str = "dangerously_allow_mock_testing_request_params";

// LiteLLM's REQUIRED_BODY_PARAMS_BY_ROUTE: anthropic_messages has no entry
const REQUIRED_BODY_PARAMS: [(&str, &[&str]); 4] = [
    ("acompletion", &["messages"]),
    ("aembedding", &["input"]),
    ("aresponses", &["input"]),
    (
        "acreate_batch",
        &["input_file_id", "endpoint", "completion_window"],
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallStepError {
    #[error("{route}: Missing required parameter: '{param}'.")]
    MissingParam { route: String, param: String },
    #[error("{}", mock_testing_disabled_message(.names.as_slice()))]
    MockParamsDisabled { names: Vec<String> },
}

fn mock_testing_disabled_message(names: &[String]) -> String {
    format!(
        "Mock testing request params are disabled on this proxy: {}. An admin can enable them by \
         setting `general_settings.{MOCK_TESTING_CONFIG_KEY}: true` in config.yaml. This setting \
         cannot be changed from the Admin UI or the API.",
        names.join(", ")
    )
}

impl CallStepError {
    // ProxyMissingRequiredParamError's ProxyException fields; the mock gate raises an HTTPException instead
    #[must_use]
    pub const fn error_type(&self) -> Option<&'static str> {
        match self {
            Self::MissingParam { .. } => Some("invalid_request_error"),
            Self::MockParamsDisabled { .. } => None,
        }
    }

    #[must_use]
    pub fn param(&self) -> Option<&str> {
        match self {
            Self::MissingParam { param, .. } => Some(param),
            Self::MockParamsDisabled { .. } => None,
        }
    }

    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        match self {
            Self::MissingParam { .. } => Some("400"),
            Self::MockParamsDisabled { .. } => None,
        }
    }
}

pub fn required_param_missing(route_type: &str, data: &JsonMap) -> Result<(), CallStepError> {
    let params: &[&str] = REQUIRED_BODY_PARAMS
        .iter()
        .find(|(route, _params)| *route == route_type)
        .map_or(&[], |(_route, params)| *params);
    let missing = params
        .iter()
        .find(|param| data.get(**param).is_none_or(JsonValue::is_null));
    let Some(param) = missing else {
        return Ok(());
    };
    // ROUTE_ENDPOINT_MAPPING has no anthropic_messages entry, so the route type is the name there
    let route = route_endpoint(route_type).unwrap_or(route_type);
    Err(CallStepError::MissingParam {
        route: route.to_owned(),
        param: (*param).to_owned(),
    })
}

pub fn mock_params_disallowed(data: &JsonMap, allowed: bool) -> Result<(), CallStepError> {
    if allowed {
        return Ok(());
    }
    let names: Vec<String> = GATED_MOCK_PARAM_NAMES
        .iter()
        .filter(|name| data.contains_key(**name))
        .copied()
        .map(str::to_owned)
        .collect();
    if names.is_empty() {
        return Ok(());
    }
    Err(CallStepError::MockParamsDisabled { names })
}
