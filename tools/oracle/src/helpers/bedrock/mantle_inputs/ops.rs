use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_bedrock::mantle_inputs::capability::{base_segment, supports_responses};
use betterlite_bedrock::mantle_inputs::chat_endpoint::{ChatEndpointParams, chat_endpoint_inputs};
use betterlite_bedrock::mantle_inputs::credentials::resolve_bearer_token;
use betterlite_bedrock::mantle_inputs::records::MantleGlobals;
use betterlite_bedrock::mantle_inputs::region::{resolve_region, split_region_prefix};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_config::loader::ConfigLoader;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::{BearerCase, ChatCase, Input, RegionCase};
use crate::helpers::{HelperError, HelperResult};

const FIXTURE_CONFIG: &str =
    "model_list:\n  - model_name: fixture\n    litellm_params: {model: bedrock/fixture}\n";

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "MantleInputError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn globals(environment: &BTreeMap<String, String>) -> Result<MantleGlobals, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(FIXTURE_CONFIG)
    .map_err(failure)?;
    let mantle = &config.sections.bedrock_mantle_inputs;
    let aws = &config.sections.bedrock_aws_endpoints;
    Ok(MantleGlobals {
        mantle_api_key: mantle.api_key.clone(),
        mantle_api_base: mantle.api_base.clone(),
        mantle_region: mantle.region.clone(),
        aws_bearer_token: config
            .sections
            .bedrock_settings
            .deployments
            .first()
            .and_then(|deployment| deployment.bearer_token.clone()),
        aws_region_name: aws.aws_region_name.clone(),
        aws_region: aws.aws_region.clone(),
    })
}

fn bearer_precedence(cases: &[BearerCase]) -> HelperResult {
    let mut tokens = Vec::with_capacity(cases.len());
    for case in cases {
        let globals = globals(&case.environment)?;
        let api_key = case.api_key.as_deref().map(SecretString::from);
        tokens.push(
            resolve_bearer_token(
                api_key.as_ref(),
                globals.mantle_api_key.as_ref(),
                globals.aws_bearer_token.as_ref(),
            )
            .map(|selected| selected.expose_secret().to_owned()),
        );
    }
    Ok(json!({"tokens": tokens}))
}

fn region_prefixes(models: &[String]) -> Value {
    let prefixes: Vec<Value> = models
        .iter()
        .map(|model| {
            let (region, base) = split_region_prefix(model);
            json!([region, base])
        })
        .collect();
    json!({"prefixes": prefixes})
}

fn region_resolution(cases: &[RegionCase]) -> HelperResult {
    let mut regions = Vec::with_capacity(cases.len());
    for case in cases {
        let globals = globals(&case.environment)?;
        let resolved = resolve_region(
            case.aws_region_name.as_deref(),
            case.api_base.as_deref(),
            &globals,
        );
        regions.push(resolved.map_or_else(
            |_error| json!({"error": true}),
            |region| json!({"region": region}),
        ));
    }
    Ok(json!({"regions": regions}))
}

fn model_capabilities(model_cost: &ParamMap, models: &[Option<String>]) -> Value {
    let capabilities: Vec<Value> = models
        .iter()
        .map(|model| {
            let name = model.as_deref();
            json!({
                "supports_responses": supports_responses(name, model_cost),
                "base_segment": base_segment(name, model_cost),
            })
        })
        .collect();
    json!({"capabilities": capabilities})
}

fn chat_inputs(model_cost: &ParamMap, cases: &[ChatCase]) -> HelperResult {
    let mut inputs = Vec::with_capacity(cases.len());
    for case in cases {
        let globals = globals(&case.environment)?;
        let api_key = case.api_key.as_deref().map(SecretString::from);
        let params = ChatEndpointParams {
            model: case.model.as_deref(),
            api_base: case.api_base.as_deref(),
            api_key: api_key.as_ref(),
            aws_region_name: case.aws_region_name.as_deref(),
        };
        let outcome = chat_endpoint_inputs(params, &globals, model_cost);
        inputs.push(outcome.map_or_else(
            |_error| json!({"error": true}),
            |resolved| {
                json!({
                    "api_base": resolved.api_base,
                    "api_key": resolved.api_key.map(|selected| selected.expose_secret().to_owned()),
                })
            },
        ));
    }
    Ok(json!({"inputs": inputs}))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::BearerPrecedence { cases } => bearer_precedence(cases),
        Input::RegionPrefix { models } => Ok(region_prefixes(models)),
        Input::RegionResolution { cases } => region_resolution(cases),
        Input::ModelCapabilities { model_cost, models } => {
            Ok(model_capabilities(model_cost, models))
        }
        Input::ChatEndpoint { model_cost, cases } => chat_inputs(model_cost, cases),
    }
}
