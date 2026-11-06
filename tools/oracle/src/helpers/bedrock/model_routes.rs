use betterlite_bedrock::model_routes::catalog::{
    RESPONSE_FORMAT_FLAG, TOOLS_REASONING_FLAG, chat_completions_is_default, openai_gpt_version,
    price_map_flag,
};
use betterlite_bedrock::model_routes::model_info::{BedrockModelInfo, ModelInfo as _};
use betterlite_bedrock::model_routes::request::{
    request_needs_converse, response_format_needs_converse,
};
use betterlite_bedrock::model_routes::{
    CHAT_COMPLETIONS_PREFIX, CLAUDE_PLATFORM_PREFIX, CONVERSE_PREFIX, MANTLE_PREFIX,
    ModelRouteSelector, ParamMap, ParamValue, get_bedrock_tool_name, get_claude_platform_model,
    has_route_prefix, map_claude_platform_auth, without_bedrock_route_prefix,
};
use indexmap::{IndexMap, IndexSet};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    price_map: ParamMap,
    #[serde(default)]
    converse_models: Vec<String>,
    #[serde(default)]
    request_metadata_fields: Vec<String>,
    #[serde(default)]
    tool_names: IndexMap<String, String>,
    step: Step,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Routes {
        #[serde(default)]
        models: Vec<String>,
        #[serde(default)]
        requests: Vec<RouteRequest>,
    },
    Capabilities {
        models: Vec<String>,
        #[serde(default)]
        malformed: Vec<String>,
    },
    RequestNeedsConverse {
        probes: Vec<RequestProbe>,
    },
    ResponseFormat {
        probes: Vec<ResponseProbe>,
    },
    ToolNames {
        names: Vec<String>,
    },
    Prefixes {
        models: Vec<String>,
        #[serde(default)]
        workspace: Vec<WorkspaceProbe>,
    },
    ModelInfo {
        probes: Vec<ModelInfoProbe>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteRequest {
    model: String,
    #[serde(default)]
    request_params: ParamMap,
    #[serde(default)]
    additional_drop_params: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestProbe {
    model: String,
    request_params: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseProbe {
    model: String,
    #[serde(default)]
    response_format: Option<ParamValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceProbe {
    passed: ParamMap,
    #[serde(default)]
    optional: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelInfoProbe {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: IndexMap<String, String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let converse_models: IndexSet<String> = input.converse_models.iter().cloned().collect();
        let owned = !input.request_metadata_fields.is_empty();
        let selector = ModelRouteSelector::new(&input.price_map, &converse_models, owned)
            .map_err(|error| route_error(&error))?;
        match &input.step {
            Step::Routes { models, requests } => routes(&selector, models, requests),
            Step::Capabilities { models, malformed } => capabilities(&selector, models, malformed),
            Step::RequestNeedsConverse { probes } => request_fallbacks(&selector, probes),
            Step::ResponseFormat { probes } => response_formats(&selector, probes),
            Step::ToolNames { names } => Ok(tool_names(&input.tool_names, names)),
            Step::Prefixes { models, workspace } => Ok(prefixes(models, workspace)),
            Step::ModelInfo { probes } => Ok(model_info(probes)),
        }
    }
}

fn route_error(error: &betterlite_bedrock::model_routes::RouteError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RouteError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn routes(
    selector: &ModelRouteSelector<'_>,
    models: &[String],
    requests: &[RouteRequest],
) -> HelperResult {
    let mut plain = Vec::new();
    for model in models {
        plain.push(json!([
            model,
            selector
                .route(model, None)
                .map_err(|error| route_error(&error))?
        ]));
    }
    let mut with_requests = Vec::new();
    for request in requests {
        let before = request.request_params.clone();
        let route = selector
            .route_for_request(
                &request.model,
                &request.request_params,
                request.additional_drop_params.as_deref(),
            )
            .map_err(|error| route_error(&error))?;
        let unchanged = request.request_params == before;
        with_requests.push(json!([request.model, route, unchanged]));
    }
    Ok(json!({"models": plain, "requests": with_requests}))
}

fn capabilities(
    selector: &ModelRouteSelector<'_>,
    models: &[String],
    malformed: &[String],
) -> HelperResult {
    let mut facts = Vec::new();
    for model in models {
        let version = openai_gpt_version(selector, model).map_err(|error| route_error(&error))?;
        facts.push(json!([
            model,
            {
                "version": version,
                "is_openai_gpt": version.is_some(),
                "is_default": chat_completions_is_default(selector, model).map_err(|error| route_error(&error))?,
                "serves_tools_with_reasoning": price_map_flag(selector, model, TOOLS_REASONING_FLAG).map_err(|error| route_error(&error))?,
                "enforces_response_format": price_map_flag(selector, model, RESPONSE_FORMAT_FLAG).map_err(|error| route_error(&error))?,
            }
        ]));
    }
    let mut raised = Vec::new();
    for model in malformed {
        let result = chat_completions_is_default(selector, model);
        raised.push(json!([model, result.is_err()]));
    }
    Ok(json!({"models": facts, "malformed": raised}))
}

fn request_fallbacks(selector: &ModelRouteSelector<'_>, probes: &[RequestProbe]) -> HelperResult {
    let mut results = Vec::new();
    for probe in probes {
        let needed = request_needs_converse(selector, &probe.model, &probe.request_params)
            .map_err(|error| route_error(&error))?;
        results.push(json!([probe.model, needed]));
    }
    Ok(Value::Array(results))
}

fn response_formats(selector: &ModelRouteSelector<'_>, probes: &[ResponseProbe]) -> HelperResult {
    probes
        .iter()
        .map(|probe| {
            let needed = response_format_needs_converse(
                selector,
                &probe.model,
                probe.response_format.as_ref(),
            )
            .map_err(|error| route_error(&error))?;
            Ok(json!([probe.model, needed]))
        })
        .collect::<Result<Value, Box<HelperError>>>()
}

fn tool_names(mappings: &IndexMap<String, String>, names: &[String]) -> Value {
    let mut results = Vec::new();
    for name in names {
        results.push(json!([name, get_bedrock_tool_name(mappings, name)]));
    }
    Value::Array(results)
}

fn prefixes(models: &[String], workspace: &[WorkspaceProbe]) -> Value {
    let mut results = Vec::new();
    for model in models {
        results.push(json!([
            model,
            {
                "without_route_prefix": without_bedrock_route_prefix(model),
                "claude_platform_model": get_claude_platform_model(model),
                "explicit_converse": has_route_prefix(model, CONVERSE_PREFIX),
                "explicit_claude_platform": has_route_prefix(model, CLAUDE_PLATFORM_PREFIX),
                "explicit_mantle": has_route_prefix(model, MANTLE_PREFIX),
                "chat_completions": has_route_prefix(model, CHAT_COMPLETIONS_PREFIX),
            }
        ]));
    }
    let mut mapped = Vec::new();
    for probe in workspace {
        let mut optional = probe.optional.clone();
        map_claude_platform_auth(&probe.passed, &mut optional);
        mapped.push(json!(optional));
    }
    json!({"models": results, "workspace_keys": mapped})
}

fn model_info(probes: &[ModelInfoProbe]) -> Value {
    let mut results = Vec::new();
    for probe in probes {
        let api_key = probe.api_key.clone().map(SecretString::from);
        let resolved_key =
            BedrockModelInfo::get_api_key(api_key).map(|key| key.expose_secret().to_owned());
        results.push(json!({
            "api_base": BedrockModelInfo::get_api_base(probe.api_base.clone()),
            "api_key": resolved_key,
            "headers": probe.headers,
            "models": BedrockModelInfo::get_models(),
        }));
    }
    Value::Array(results)
}
