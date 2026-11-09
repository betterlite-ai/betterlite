use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails, Usage};
use betterlite_models::zero_cost::{
    ZeroCostDiagnostic, diagnose_zero_cost, used_pricing_keys, zero_cost_warning,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

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
    let pricing = pricing_from_value(&probe.pricing)?;
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

fn usage_from_value(value: &Value) -> Result<Usage, Box<crate::helpers::HelperError>> {
    Ok(Usage {
        prompt_tokens: token(value.get("prompt_tokens"), "prompt_tokens")?,
        completion_tokens: token(value.get("completion_tokens"), "completion_tokens")?,
        total_tokens: token(value.get("total_tokens"), "total_tokens")?,
        prompt_tokens_details: prompt_details(value.get("prompt_tokens_details"))?,
        completion_tokens_details: completion_details(value.get("completion_tokens_details"))?,
        numeric_extras: std::collections::BTreeMap::new(),
    })
}

fn token(
    value: Option<&Value>,
    name: &'static str,
) -> Result<i64, Box<crate::helpers::HelperError>> {
    match value {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| failure(format!("{name} must be an integer"))),
        Some(_) => Err(failure(format!("{name} must be an integer"))),
    }
}

fn prompt_details(
    value: Option<&Value>,
) -> Result<Option<PromptTokensDetails>, Box<crate::helpers::HelperError>> {
    let Some(found) = object_or_absent(value, "prompt_tokens_details")? else {
        return Ok(None);
    };
    Ok(Some(PromptTokensDetails {
        text_tokens: detail_count(found, "text_tokens"),
        audio_tokens: detail_count(found, "audio_tokens"),
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
        google_maps_grounding_requests: None,
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cached_tokens: None,
        cached_tokens_details: None,
    }))
}

fn completion_details(
    value: Option<&Value>,
) -> Result<Option<CompletionTokensDetails>, Box<crate::helpers::HelperError>> {
    let Some(found) = object_or_absent(value, "completion_tokens_details")? else {
        return Ok(None);
    };
    Ok(Some(CompletionTokensDetails {
        text_tokens: detail_count(found, "text_tokens"),
        audio_tokens: detail_count(found, "audio_tokens"),
        image_tokens: None,
        video_tokens: None,
        reasoning_tokens: None,
        accepted_prediction_tokens: None,
        rejected_prediction_tokens: None,
    }))
}

fn object_or_absent<'value>(
    value: Option<&'value Value>,
    name: &'static str,
) -> Result<Option<&'value Map<String, Value>>, Box<crate::helpers::HelperError>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(found)) => Ok(Some(found)),
        Some(_) => Err(failure(format!("{name} must be an object"))),
    }
}

fn detail_count(details: &Map<String, Value>, name: &str) -> Option<i64> {
    match details.get(name) {
        Some(Value::Number(number)) => number.as_i64(),
        None | Some(_) => None,
    }
}

fn pricing_from_value(
    value: &Value,
) -> Result<IndexMap<String, Json>, Box<crate::helpers::HelperError>> {
    let parsed: Json = serde_json::from_value(value.clone()).map_err(failure)?;
    let Json::Object(fields) = parsed else {
        return Err(failure("pricing must be an object"));
    };
    Ok(fields)
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
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
