use std::sync::Arc;
use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::counters::{UsageCounters, UsageLabels};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use super::usage_object::object;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::failure::text_field;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::LabelFactoryError;
use betterlite_models::decisions::Json;

pub fn increment_mcp_tool_calls<C>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    response_cost: Decimal,
    now: Instant,
) -> Result<(), PrometheusUsageError<<C::Counter as RemovableMetric>::Error>>
where
    C: UsageCounters,
{
    let Some(metadata) = object(payload.metadata.mcp_tool_call_metadata.as_ref()) else {
        return Ok(());
    };
    let (values, context) = mcp_label_context(metadata, labels)?;
    let mcp_labels = UsageLabels {
        values: &values,
        context: &context,
        settings: labels.settings,
    };
    counters.inc(
        UsageMetric::McpToolCalls,
        mcp_labels,
        MetricValue::Finite(Decimal::ONE),
        now,
    )?;
    if response_cost > Decimal::ZERO {
        counters.inc(
            UsageMetric::McpToolCallSpend,
            mcp_labels,
            MetricValue::Finite(response_cost),
            now,
        )?;
    }
    Ok(())
}

fn mcp_label_context(
    metadata: &IndexMap<String, Json>,
    labels: UsageLabels<'_>,
) -> Result<(UserApiKeyLabelValues, PrometheusLabelFactoryContext), LabelFactoryError> {
    let values = mcp_values(metadata, labels.values);
    let context = PrometheusLabelFactoryContext::with_resolver(
        &values,
        labels.settings,
        Arc::clone(&labels.context.resolver),
    )
    .map_err(LabelFactoryError::from)?;
    Ok((values, context))
}

fn mcp_values(
    metadata: &IndexMap<String, Json>,
    request: &UserApiKeyLabelValues,
) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = [
        ("mcp_tool_name", text_field(metadata, "name")),
        ("mcp_server_name", text_field(metadata, "mcp_server_name")),
        ("hashed_api_key", request.hashed_api_key.as_deref()),
        ("api_key_alias", request.api_key_alias.as_deref()),
        ("team", request.team.as_deref()),
        ("team_alias", request.team_alias.as_deref()),
        ("user", request.user.as_deref()),
        ("end_user", request.end_user.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value.map(|text| (name.to_owned(), LabelDumpValue::String(text.to_owned())))
    })
    .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}
