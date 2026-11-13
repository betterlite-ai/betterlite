mod capability;
mod lyria;
mod records;

use betterlite_vertex::model_policy::partners::{
    is_vertex_partner_model, should_use_openai_handler,
};
use betterlite_vertex::model_policy::publisher::is_self_deployed_endpoint;
use betterlite_vertex::model_policy::route::{RouteParams, get_vertex_ai_model_route};
use betterlite_vertex::model_urls::route::VertexModelRoute;
use betterlite_vertex::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};
use jiff::tz::TimeZone;
use serde_json::{Value, json};

use crate::helpers::HelperError;

use capability::capabilities;
use lyria::lyria;
use records::{labels, metadata};

use super::input::{Input, RouteCase};

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    match input {
        Input::Route { cases } => Ok(routes(cases)),
        Input::Partner { models } => Ok(partners(models)),
        Input::Capabilities { cases } => Ok(capabilities(cases)),
        Input::Lyria { cases } => Ok(lyria(cases)),
        Input::Labels { cases } => Ok(labels(cases)),
        Input::Metadata {
            logged_objects,
            call_details,
        } => Ok(metadata(logged_objects, call_details)),
        Input::Timestamp { zone, values } => timestamps(zone, values),
    }
}

fn routes(cases: &[RouteCase]) -> Value {
    let routes: Vec<&str> = cases
        .iter()
        .map(|case| {
            let params = RouteParams {
                base_model: case.base_model.as_deref(),
                api_base: case.api_base.as_deref(),
            };
            route_name(get_vertex_ai_model_route(&case.model, &params))
        })
        .collect();
    json!({"routes": routes})
}

const fn route_name(route: VertexModelRoute) -> &'static str {
    match route {
        VertexModelRoute::PartnerModels => "partner_models",
        VertexModelRoute::Gemini => "gemini",
        VertexModelRoute::Gemma => "gemma",
        VertexModelRoute::Bge => "bge",
        VertexModelRoute::ModelGarden => "model_garden",
        VertexModelRoute::NonGemini => "non_gemini",
        VertexModelRoute::OpenAi => "openai",
        VertexModelRoute::AgentEngine => "agent_engine",
    }
}

fn partners(models: &[String]) -> Value {
    let entries: Vec<Value> = models
        .iter()
        .map(|model| {
            json!({
                "model": model,
                "partner": is_vertex_partner_model(model),
                "openai_handler": should_use_openai_handler(model),
                "self_deployed": is_self_deployed_endpoint(model),
            })
        })
        .collect();
    json!({"models": entries})
}

fn timestamps(zone: &str, values: &[String]) -> Result<Value, Box<HelperError>> {
    let time_zone = TimeZone::get(zone)
        .map_err(|error| invalid_input(format!("unknown time zone {zone}: {error}")))?;
    let context = LocalTimeContext { zone: time_zone };
    let results: Vec<Value> = values
        .iter()
        .map(|value| {
            decode_vertex_timestamp(value.as_bytes(), &context).map_or_else(
                |_error| json!({"error": true}),
                |epoch| json!({"epoch": epoch}),
            )
        })
        .collect();
    Ok(json!({"timestamps": results}))
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
