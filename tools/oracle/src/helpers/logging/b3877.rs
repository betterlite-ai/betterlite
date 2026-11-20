use std::collections::BTreeMap;
use std::time::Instant;

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_logging::prometheus_usage::cache::increment_cache_metrics;
use betterlite_logging::prometheus_usage::counters::UsageLabels;
use betterlite_logging::prometheus_usage::media::increment_media_metrics;
use betterlite_logging::prometheus_usage::request_spend::{
    increment_request_and_spend, increment_zero_cost_requests,
};
use betterlite_logging::prometheus_usage::tokens::{
    increment_token_detail_metrics, increment_token_metrics,
};
use betterlite_logging::prometheus_usage::usage_object::{
    resolve_cache_write_tokens, resolve_provider_cache_tokens,
};
use betterlite_models::decisions::Json;
use betterlite_models::zero_cost::{ZeroCostDiagnostic, ZeroCostReason};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use serde_json::json;

use output::events_value;
use probe::ProbeCounters;

mod output;
mod payload;
mod probe;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    function: String,
    #[serde(default)]
    values: BTreeMap<String, Option<String>>,
    #[serde(default)]
    supported_labels: BTreeMap<String, Vec<String>>,
    response_cost: Option<Json>,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    cache_hit: Option<bool>,
    usage_object: Option<Json>,
    provider_usage: Option<Json>,
    prompt_details: Option<Json>,
}

#[derive(Clone, Copy)]
enum Emit {
    RequestAndSpend,
    ZeroCost,
    TokenMetrics,
    TokenDetail,
    Media,
    Cache,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "request_and_spend" => emit(input, Emit::RequestAndSpend),
            "zero_cost" => emit(input, Emit::ZeroCost),
            "token_metrics" => emit(input, Emit::TokenMetrics),
            "token_detail" => emit(input, Emit::TokenDetail),
            "media" => emit(input, Emit::Media),
            "cache" => emit(input, Emit::Cache),
            "provider_cache_tokens" => provider_cache_tokens(input),
            "cache_write_tokens" => Ok(json!({
                "cache_write_tokens": resolve_cache_write_tokens(input.prompt_details.as_ref()),
            })),
            _ => Err(helper_error("InvalidInput", "unknown function")),
        }
    }
}

fn emit(input: &Input, function: Emit) -> HelperResult {
    let counters = ProbeCounters::new(&input.supported_labels);
    let labels = labels_for(input)?;
    let outcome = match function {
        Emit::RequestAndSpend => {
            let payload = payload::payload(input.response_cost.as_ref(), (0, 0, 0), None, None)?;
            increment_request_and_spend(&counters, &payload, labels.usage_labels(), Instant::now())
        }
        Emit::ZeroCost => {
            let diagnostic = ZeroCostDiagnostic {
                reason: ZeroCostReason::MissingPricingKey,
                pricing_model: String::new(),
                missing_pricing_keys: Vec::new(),
            };
            increment_zero_cost_requests(&counters, Some(&diagnostic), labels.usage_labels())
        }
        Emit::TokenMetrics => {
            let counts = (
                input.total_tokens,
                input.prompt_tokens,
                input.completion_tokens,
            );
            let payload = payload::payload(None, counts, None, input.usage_object.as_ref())?;
            increment_token_metrics(&counters, &payload, labels.usage_labels(), Instant::now())
        }
        Emit::TokenDetail => {
            let payload = payload::payload(None, (0, 0, 0), None, input.usage_object.as_ref())?;
            increment_token_detail_metrics(
                &counters,
                &payload,
                labels.usage_labels(),
                Instant::now(),
            )
        }
        Emit::Media => {
            let payload = payload::payload(None, (0, 0, 0), None, input.usage_object.as_ref())?;
            increment_media_metrics(&counters, &payload, labels.usage_labels(), Instant::now())
        }
        Emit::Cache => {
            let counts = (input.total_tokens, 0, 0);
            let payload =
                payload::payload(None, counts, input.cache_hit, input.usage_object.as_ref())?;
            increment_cache_metrics(&counters, &payload, labels.usage_labels(), Instant::now())
        }
    };
    outcome.map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(events_value(&counters))
}

fn provider_cache_tokens(input: &Input) -> HelperResult {
    let Some(Json::Object(usage)) = input.provider_usage.as_ref() else {
        return Err(helper_error(
            "InvalidInput",
            "provider_usage must be an object",
        ));
    };
    let (read, creation) = resolve_provider_cache_tokens(usage);
    Ok(json!({ "read": read, "creation": creation }))
}

struct LabelFixture {
    values: UserApiKeyLabelValues,
    context: PrometheusLabelFactoryContext,
    settings: PrometheusLabelSettings,
}

impl LabelFixture {
    fn usage_labels(&self) -> UsageLabels<'_> {
        UsageLabels {
            values: &self.values,
            context: &self.context,
            settings: &self.settings,
        }
    }
}

fn labels_for(input: &Input) -> Result<LabelFixture, Box<HelperError>> {
    let values = label_values(&input.values);
    let settings = PrometheusLabelSettings::with_defaults();
    let context = context_for(&values, &settings)?;
    Ok(LabelFixture {
        values,
        context,
        settings,
    })
}

fn context_for(
    values: &UserApiKeyLabelValues,
    settings: &PrometheusLabelSettings,
) -> Result<PrometheusLabelFactoryContext, Box<HelperError>> {
    PrometheusLabelFactoryContext::new(values, settings, EndUserCostTrackingSettings::default())
        .map_err(|error| helper_error("InvalidInput", &error.to_string()))
}

fn label_values(values: &BTreeMap<String, Option<String>>) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = values
        .iter()
        .map(|(name, value)| {
            let dumped = value.as_ref().map_or(LabelDumpValue::Null, |text| {
                LabelDumpValue::String(text.clone())
            });
            (name.clone(), dumped)
        })
        .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
