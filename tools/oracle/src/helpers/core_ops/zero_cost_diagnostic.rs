use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::decisions::Json;
use betterlite_models::zero_cost::{
    TokenUsage, ZeroCostDiagnostic, diagnose_zero_cost, used_pricing_keys, zero_cost_warning,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Display;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    probes: Vec<Probe>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    usage: Value,
    pricing_model: String,
    pricing: Value,
    #[serde(default)]
    calculation_failed: bool,
    model: String,
    #[serde(default)]
    model_group: Option<String>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input
            .probes
            .iter()
            .map(run_probe)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(outcomes))
    }
}

fn run_probe(probe: &Probe) -> Result<Value, Box<crate::helpers::HelperError>> {
    let usage = usage_from_value(&probe.usage)?;
    let pricing = object_mapping(&probe.pricing, "pricing")?;
    let used_keys = used_pricing_keys(&usage);
    let diagnostic = diagnose_zero_cost(
        &usage,
        &probe.pricing_model,
        &pricing,
        probe.calculation_failed,
    );
    let warning = diagnostic.as_ref().map(|found| {
        zero_cost_warning(
            found,
            probe.model_group.as_deref(),
            &probe.model,
            probe.custom_llm_provider.as_deref(),
            &usage,
        )
    });
    Ok(json!({
        "used_keys": used_keys,
        "diagnostic": diagnostic.as_ref().map(diagnostic_value),
        "warning": warning,
    }))
}

fn diagnostic_value(diagnostic: &ZeroCostDiagnostic) -> Value {
    json!({
        "reason": diagnostic.reason.label(),
        "pricing_model": diagnostic.pricing_model,
        "missing_pricing_keys": diagnostic.missing_pricing_keys,
    })
}

fn usage_from_value(value: &Value) -> Result<TokenUsage, Box<crate::helpers::HelperError>> {
    let converted = usage_from_mapping(&object_mapping(value, "usage")?).map_err(failure)?;
    Ok(TokenUsage {
        prompt_tokens: converted.prompt_tokens,
        completion_tokens: converted.completion_tokens,
        prompt_audio_tokens: audio_tokens(converted.prompt_tokens_details.as_ref()),
        completion_audio_tokens: audio_tokens(converted.completion_tokens_details.as_ref()),
    })
}

fn audio_tokens(details: Option<&IndexMap<String, Json>>) -> i64 {
    match details.and_then(|found| found.get("audio_tokens")) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}

fn object_mapping(
    value: &Value,
    name: &str,
) -> Result<IndexMap<String, Json>, Box<crate::helpers::HelperError>> {
    let parsed: Json = serde_json::from_value(value.clone()).map_err(failure)?;
    let Json::Object(fields) = parsed else {
        return Err(failure(format!("{name} must be an object")));
    };
    Ok(fields)
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
