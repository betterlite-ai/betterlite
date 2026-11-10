use betterlite_logging::passthrough_logging_state::cost_per_request::set_cost_per_request;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_logging_state::usage::{
    PassThroughCallState, UpstreamReportedUsage, apply_upstream_reported_usage,
    has_upstream_reported_usage, parse_response_cost, parse_total_tokens,
    parse_upstream_reported_usage,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::ParseUsage { headers } => parse_usage(headers)?,
            Input::ParseCost { raw } => json!(parse_response_cost(raw.as_deref())),
            Input::ParseTokens { raw } => json!(parse_total_tokens(raw.as_deref())),
            Input::ApplyUsage { headers, state } => apply_usage(headers, state)?,
            Input::SetCostPerRequest {
                state,
                kwargs_response_cost,
                cost_per_request,
            } => set_cost(state, *kwargs_response_cost, *cost_per_request)?,
            Input::Route { routes } => Value::Array(routes.iter().map(route_json).collect()),
        };
        Ok(result)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ParseUsage {
        headers: Vec<(String, String)>,
    },
    ParseCost {
        #[serde(default)]
        raw: Option<String>,
    },
    ParseTokens {
        #[serde(default)]
        raw: Option<String>,
    },
    ApplyUsage {
        headers: Vec<(String, String)>,
        #[serde(default)]
        state: StateInput,
    },
    SetCostPerRequest {
        #[serde(default)]
        state: StateInput,
        #[serde(default)]
        // ast-grep-ignore: no-float-money
        kwargs_response_cost: Option<f64>,
        #[serde(default)]
        // ast-grep-ignore: no-float-money
        cost_per_request: Option<f64>,
    },
    Route {
        routes: Vec<RouteInput>,
    },
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct StateInput {
    #[serde(default)]
    upstream_reported_usage: Option<ReportedInput>,
    #[serde(default)]
    // ast-grep-ignore: no-float-money
    response_cost: Option<f64>,
    #[serde(default)]
    combined_usage_total_tokens: Option<i64>,
}

impl StateInput {
    fn to_call_state(&self) -> PassThroughCallState {
        PassThroughCallState {
            upstream_reported_usage: self.upstream_reported_usage.as_ref().map(|reported| {
                UpstreamReportedUsage {
                    response_cost: reported.response_cost,
                    total_tokens: reported.total_tokens,
                }
            }),
            response_cost: self.response_cost,
            combined_usage_total_tokens: self.combined_usage_total_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReportedInput {
    #[serde(default)]
    // ast-grep-ignore: no-float-money
    response_cost: Option<f64>,
    #[serde(default)]
    total_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RouteInput {
    url_route: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
}

fn parse_usage(pairs: &[(String, String)]) -> Result<Value, Box<crate::helpers::HelperError>> {
    let headers = build_headers(pairs)?;
    let reported = parse_upstream_reported_usage(&headers);
    Ok(reported.map_or(Value::Null, reported_json))
}

fn apply_usage(
    pairs: &[(String, String)],
    state: &StateInput,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let headers = build_headers(pairs)?;
    let mut call_state = state.to_call_state();
    let applied = apply_upstream_reported_usage(&mut call_state, &headers);
    Ok(json!({
        "upstream_reported_usage": call_state.upstream_reported_usage.map(reported_json),
        "response_cost": call_state.response_cost,
        "combined_usage_total_tokens": call_state.combined_usage_total_tokens,
        "has_upstream_reported_usage": has_upstream_reported_usage(&call_state),
        "applied": applied.map(reported_json),
    }))
}

fn set_cost(
    state: &StateInput,
    kwargs_response_cost: Option<f64>,
    cost_per_request: Option<f64>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut call_state = state.to_call_state();
    let mut kwargs_cost = kwargs_response_cost;
    let amount = cost_per_request
        .map(Decimal::try_from)
        .transpose()
        .map_err(|error| invalid(&error.to_string()))?;
    set_cost_per_request(&mut call_state, &mut kwargs_cost, amount);
    Ok(json!({
        "kwargs_response_cost": kwargs_cost,
        "state_response_cost": call_state.response_cost,
    }))
}

fn route_json(route: &RouteInput) -> Value {
    let url_route = route.url_route.as_str();
    let provider = route.custom_llm_provider.as_deref();
    json!({
        "anthropic": PassThroughEndpointLogging::is_anthropic_route(url_route),
        "cohere": PassThroughEndpointLogging::is_cohere_route(url_route),
        "assemblyai": PassThroughEndpointLogging::is_assemblyai_route(url_route),
        "comprehend_medical": PassThroughEndpointLogging::is_comprehend_medical_route(provider),
        "azure_speech": PassThroughEndpointLogging::is_azure_speech_route(provider),
        "transcribe": PassThroughEndpointLogging::is_transcribe_route(provider),
        "typesafe": PassThroughEndpointLogging::is_typesafe_route(provider),
        "openrouter_decisions": PassThroughEndpointLogging::is_openrouter_decisions_route(url_route, provider),
        "langfuse": PassThroughEndpointLogging::is_langfuse_route(url_route),
        "vertex_ai_live": PassThroughEndpointLogging::is_vertex_ai_live_route(url_route),
        "cursor": PassThroughEndpointLogging::is_cursor_route(url_route, provider),
    })
}

fn reported_json(usage: UpstreamReportedUsage) -> Value {
    json!({
        "response_cost": usage.response_cost,
        "total_tokens": usage.total_tokens,
    })
}

fn build_headers(
    pairs: &[(String, String)],
) -> Result<HeaderMap, Box<crate::helpers::HelperError>> {
    let mut headers = HeaderMap::new();
    for (raw_name, raw_value) in pairs {
        let name = HeaderName::from_bytes(raw_name.as_bytes())
            .map_err(|error| invalid(&error.to_string()))?;
        let value =
            HeaderValue::from_str(raw_value).map_err(|error| invalid(&error.to_string()))?;
        headers.append(name, value);
    }
    Ok(headers)
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
