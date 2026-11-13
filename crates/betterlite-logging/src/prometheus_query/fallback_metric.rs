use super::PrometheusQuery;
use super::clock::Clock;
use super::error::QueryError;
use super::get_metric::get_metric_from_prometheus;
use super::response_records::PrometheusRangeSample;
use super::transport::Transport;
use rust_decimal::Decimal;

const FALLBACK_METRICS: [&str; 2] = [
    "litellm_deployment_successful_fallbacks_total",
    "litellm_deployment_failed_fallbacks_total",
];

pub async fn fallback_from_prometheus<Fetch, Time>(
    query: &PrometheusQuery<Fetch, Time>,
) -> Result<String, QueryError<Fetch::Error>>
where
    Fetch: Transport,
    Time: Clock,
{
    let selected_instance = query
        .settings
        .selected_instance
        .as_deref()
        .filter(|instance| !instance.is_empty());
    let mut response_message = String::new();
    for metric in FALLBACK_METRICS {
        let series_list = get_metric_from_prometheus(query, metric).await?;
        response_message.push_str(&summarize(&series_list, selected_instance)?);
    }
    Ok(response_message)
}

fn summarize<Failure>(
    series_list: &[PrometheusRangeSample],
    selected_instance: Option<&str>,
) -> Result<String, QueryError<Failure>> {
    let mut lines = Vec::new();
    for series in series_list {
        let Some(most_recent) = series.values.first() else {
            continue;
        };
        let instance = series.metric.get("instance").map(String::as_str);
        if selected_instance.is_some_and(|wanted| instance != Some(wanted)) {
            continue;
        }
        let Some(number) = most_recent.numeric_value() else {
            return Err(QueryError::SampleValue {
                value: most_recent.value.clone(),
            });
        };
        let value = if number.trunc().is_zero() {
            Decimal::ZERO
        } else {
            number.trunc()
        };
        let primary_model = series
            .metric
            .get("primary_model")
            .map_or("Unknown", String::as_str);
        let fallback_model = series
            .metric
            .get("fallback_model")
            .map_or("Unknown", String::as_str);
        lines.push(format!(
            "`{value} successful fallback requests` with primary model=`{primary_model}` -> fallback model=`{fallback_model}`\n"
        ));
    }
    Ok(lines.concat())
}

mod tests;
