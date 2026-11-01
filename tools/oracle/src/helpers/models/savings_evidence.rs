use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use betterlite_models::savings_evidence::cache_extraction::{
    UsageObject, extract_cache_creation_tokens, extract_cache_read_tokens,
};
use betterlite_models::savings_evidence::compression::compression_saved_tokens;
use betterlite_models::savings_evidence::error::SavingsEvidenceError;
use betterlite_models::savings_evidence::injection::marks_gateway_injection;
use betterlite_models::savings_evidence::metadata::Metadata;
use betterlite_models::savings_evidence::numeric::recorded_autorouter_savings;
use betterlite_models::savings_evidence::snapshot::{
    BaselineCostSnapshot, CostBreakdown, pricing_basis_from_breakdown, recorded_token_cost,
};
use betterlite_models::savings_evidence::usage::{
    RawUsageObject, baseline_usage, usage_from_spend_log,
};
use betterlite_models::token_rates::ModelInfo;
use rust_decimal::Decimal;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "autorouter")]
    Autorouter { metadata: Metadata },
    #[serde(rename = "recorded_token_cost")]
    RecordedTokenCost {
        cost_breakdown: Option<CostBreakdown>,
    },
    #[serde(rename = "cache_extraction")]
    CacheExtraction { usage_object: Option<UsageObject> },
    #[serde(rename = "baseline_usage")]
    BaselineUsage {
        usage_object: RawUsageObject,
        baseline_info: Option<ModelInfo>,
    },
    #[serde(rename = "injection")]
    Injection {
        metadata: Option<Metadata>,
        model_id: Option<String>,
    },
    #[serde(rename = "compression")]
    Compression { metadata: Metadata },
    #[serde(rename = "snapshot")]
    Snapshot {
        snapshot: Validated<BaselineCostSnapshot>,
    },
    #[serde(rename = "billed_at")]
    BilledAt { text: String },
}

// Mirrors a pydantic model's own strict rejection: a request this shape is still a valid oracle input.
#[derive(serde::Deserialize)]
#[serde(untagged)]
pub(crate) enum Validated<T> {
    Valid(T),
    Invalid(serde_json::Value),
}

impl<T> Validated<T> {
    fn get(&self) -> Result<&T, Box<crate::helpers::HelperError>> {
        match self {
            Self::Valid(value) => Ok(value),
            Self::Invalid(value) => Err(helper_error(
                "ValidationError",
                &format!("invalid snapshot: {value:?}"),
            )),
        }
    }
}

fn helper_error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}

fn decimal_value(value: Decimal) -> serde_json::Value {
    let normalized = value.normalize();
    serde_json::Value::String(normalized.to_string())
}

fn saved_tokens_value(result: Result<i64, SavingsEvidenceError>) -> crate::helpers::HelperResult {
    match result {
        Ok(count) => Ok(serde_json::json!({ "tokens_saved": count })),
        Err(error) => Err(helper_error("SavingsEvidenceError", &error.to_string())),
    }
}

fn cache_extraction_value(usage_object: Option<&UsageObject>) -> crate::helpers::HelperResult {
    let read = extract_cache_read_tokens(usage_object)
        .map_err(|error| helper_error("SavingsEvidenceError", &error.to_string()))?;
    let creation = extract_cache_creation_tokens(usage_object)
        .map_err(|error| helper_error("SavingsEvidenceError", &error.to_string()))?;
    Ok(serde_json::json!({
        "cache_read_tokens": read,
        "cache_creation_tokens": creation,
    }))
}

fn baseline_usage_value(
    usage_object: &RawUsageObject,
    baseline_info: Option<&ModelInfo>,
) -> serde_json::Value {
    let Some(usage) = usage_from_spend_log(Some(usage_object)) else {
        return serde_json::json!({ "available": false });
    };
    let normalized = baseline_usage(usage, baseline_info);
    let details = normalized.prompt_tokens_details;
    serde_json::json!({
        "available": true,
        "prompt_tokens": normalized.prompt_tokens,
        "cache_read_input_tokens": normalized.cache_read_input_tokens,
        "cache_creation_input_tokens": normalized.cache_creation_input_tokens,
        "cached_tokens": details.as_ref().map(|value| value.cached_tokens),
        "cache_write_tokens": details.as_ref().map(|value| value.cache_write_tokens),
        "text_tokens": details.as_ref().map(|value| value.text_tokens),
        "has_cache_creation_token_details": details
            .as_ref()
            .is_some_and(|value| value.cache_creation_token_details.is_some()),
    })
}

fn snapshot_value(snapshot: &Validated<BaselineCostSnapshot>) -> crate::helpers::HelperResult {
    let valid = snapshot.get()?;
    Ok(serde_json::json!({
        "ok": true,
        "model": valid.model,
        "provider": valid.provider,
        "basis": [
            &valid.basis.service_tier,
            &valid.basis.data_residency,
            &valid.basis.vertex_location,
        ],
        "actual_spend": decimal_value(valid.actual_spend),
        "actual_token_cost": valid.actual_token_cost.map(decimal_value),
        "classifier_cost": decimal_value(valid.classifier_cost),
        "has_prices": valid.prices.is_some(),
    }))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Autorouter { metadata } => Ok(serde_json::json!({
                "autorouter_savings": recorded_autorouter_savings(metadata).map(decimal_value),
            })),
            Input::RecordedTokenCost { cost_breakdown } => {
                let basis = pricing_basis_from_breakdown(cost_breakdown.as_ref());
                Ok(serde_json::json!({
                    "basis": [basis.service_tier, basis.data_residency, basis.vertex_location],
                    "recorded_token_cost": recorded_token_cost(cost_breakdown.as_ref())
                        .map(decimal_value),
                }))
            }
            Input::CacheExtraction { usage_object } => {
                cache_extraction_value(usage_object.as_ref())
            }
            Input::BaselineUsage {
                usage_object,
                baseline_info,
            } => Ok(baseline_usage_value(usage_object, baseline_info.as_ref())),
            Input::Injection { metadata, model_id } => Ok(serde_json::json!({
                "marks_gateway_injection": marks_gateway_injection(
                    metadata.as_ref(),
                    model_id.as_deref()
                ),
            })),
            Input::Compression { metadata } => {
                saved_tokens_value(compression_saved_tokens(metadata))
            }
            Input::Snapshot { snapshot } => snapshot_value(snapshot),
            Input::BilledAt { text } => Ok(serde_json::json!({
                "decoded_unix_micros": decode_billed_at(text).map(|value| value.timestamp_micros()),
            })),
        }
    }
}
