use std::fmt::Display;

use betterlite_bedrock::legacy_models::constructors::apply;
use betterlite_bedrock::legacy_models::family::{ClassAttributes, Family, LegacyConfig};
use betterlite_bedrock::legacy_models::mapping::{
    MappingRequest, map_openai_params, supported_openai_params,
};
use betterlite_bedrock::legacy_models::mistral::{MistralOutputError, output_text};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Serialize;
use serde_json::{Value, json};

use super::input::{ConstructorCases, ConstructorRow, Input, MapCase, OutputCase, ProjectionCase};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Mappings { cases } => mappings(cases),
        Input::Constructors(cases) => constructors(cases),
        Input::Projection(case) => projection(case),
        Input::OutputText(case) => output(case),
    }
}

fn failure<E>(class: &'static str, error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn attribute_value(attributes: &ClassAttributes) -> Result<Value, Box<HelperError>> {
    match serde_json::to_value(attributes) {
        Ok(value) => Ok(value),
        Err(error) => Err(failure("SerializationError", &error)),
    }
}

fn mappings(cases: &[MapCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(mapping(case)?);
    }
    Ok(json!({"results": results}))
}

fn mapping(case: &MapCase) -> Result<Value, Box<HelperError>> {
    let map_request = MappingRequest {
        family: case.family,
        non_default: &case.non_default,
        optional: case.optional.clone(),
        model: &case.model,
        drop_params: case.drop_params,
        global_drop_params: case.global_drop_params,
    };
    let mapped = serde_json::to_value(map_openai_params(map_request))
        .map_err(|error| failure("SerializationError", &error))?;
    Ok(json!({
        "supported": supported_openai_params(case.family),
        "mapped": mapped,
        "warning": Value::Null,
    }))
}

fn constructors(cases: &ConstructorCases) -> HelperResult {
    let mut results = Vec::with_capacity(cases.cases.len());
    for row in &cases.cases {
        results.push(construct(cases.default_max_tokens, row)?);
    }
    Ok(json!({"results": results}))
}

fn construct(default_max_tokens: i64, row: &ConstructorRow) -> Result<Value, Box<HelperError>> {
    match row {
        ConstructorRow::Ai21 { calls } => construct_family(Family::Ai21, default_max_tokens, calls),
        ConstructorRow::Cohere { calls } => {
            construct_family(Family::Cohere, default_max_tokens, calls)
        }
        ConstructorRow::Llama { calls } => {
            construct_family(Family::Llama, default_max_tokens, calls)
        }
        ConstructorRow::Mistral { calls } => {
            construct_family(Family::Mistral, default_max_tokens, calls)
        }
        ConstructorRow::Titan { calls } => {
            construct_family(Family::Titan, default_max_tokens, calls)
        }
        ConstructorRow::Anthropic { calls } => {
            construct_family(Family::Anthropic, default_max_tokens, calls)
        }
    }
}

fn construct_family<Values>(
    family: Family,
    default_max_tokens: i64,
    calls: &[Values],
) -> Result<Value, Box<HelperError>>
where
    Values: Serialize,
{
    let mut class = LegacyConfig::new(family, default_max_tokens);
    let mut configs = Vec::with_capacity(calls.len() + 1);
    configs.push(attribute_value(&class.get_config())?);
    for call in calls {
        apply(&mut class.attributes, call).map_err(|error| failure("ConstructorError", &error))?;
        configs.push(attribute_value(&class.get_config())?);
    }
    let initial = configs.remove(0);
    Ok(json!({"initial": initial, "after": configs}))
}

fn projection(case: &ProjectionCase) -> HelperResult {
    let class = LegacyConfig {
        family: case.family,
        attributes: case.attributes.clone(),
    };
    Ok(json!({
        "family": case.family,
        "config": attribute_value(&class.get_config())?,
    }))
}

fn output(case: &OutputCase) -> HelperResult {
    match output_text(&case.response) {
        Ok(output) => Ok(json!({
            "text": output.text,
            "finish_reason": output.finish_reason,
        })),
        Err(MistralOutputError::Unexpected(error)) => Ok(json!({
            "error": {"status": error.error.status_code, "message": error.error.message},
        })),
        Err(error) => Err(failure("MistralOutput", &error)),
    }
}
