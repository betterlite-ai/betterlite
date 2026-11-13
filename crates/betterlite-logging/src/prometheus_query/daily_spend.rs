use super::PrometheusQuery;
use super::clock::Clock;
use super::error::QueryError;
use super::fetch::fetch_series;
use super::promql_escaping::quote_promql_string_literal;
use super::response_records::PrometheusRangeSample;
use super::transport::Transport;
use chrono::{DateTime, Days, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};

const GLOBAL_SPEND_QUERY: &str = "sum(delta(litellm_spend_metric_total[1d]))";
const DAY_STEP_SECONDS: &str = "86400";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrometheusDailySpend {
    pub date: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
}

fn isoformat(moment: DateTime<Utc>) -> String {
    let base = moment.format("%Y-%m-%dT%H:%M:%S");
    match moment.timestamp_subsec_micros() {
        0 => format!("{base}+00:00"),
        micros => format!("{base}.{micros:06}+00:00"),
    }
}

fn date_from_timestamp(timestamp: Decimal) -> Option<DateTime<Utc>> {
    let rounded = timestamp.round_dp(6);
    let seconds = rounded.floor();
    let micros = rounded
        .checked_sub(seconds)?
        .checked_mul(Decimal::from(1_000_000_u32))?;
    DateTime::from_timestamp(seconds.to_i64()?, micros.to_u32()?.checked_mul(1000)?)
}

fn daily_records<Failure>(
    series_list: &[PrometheusRangeSample],
) -> Result<Vec<PrometheusDailySpend>, QueryError<Failure>> {
    let mut daily_spend = Vec::new();
    for sample in series_list.iter().flat_map(|series| &series.values) {
        let Some(date) = date_from_timestamp(sample.timestamp) else {
            return Err(QueryError::Timestamp {
                value: sample.timestamp.to_string(),
            });
        };
        let Some(spend) = sample.numeric_value() else {
            return Err(QueryError::SampleValue {
                value: sample.value.clone(),
            });
        };
        daily_spend.push(PrometheusDailySpend {
            date: isoformat(date),
            spend,
        });
    }
    Ok(daily_spend)
}

pub async fn daily_spend_from_prometheus<Fetch, Time>(
    query: &PrometheusQuery<Fetch, Time>,
    api_key: Option<&str>,
) -> Result<Vec<PrometheusDailySpend>, QueryError<Fetch::Error>>
where
    Fetch: Transport,
    Time: Clock,
{
    let endpoint = query.settings.endpoint()?;
    let end_date = query.clock.now();
    let Some(start_date) = end_date.checked_sub_days(Days::new(30)) else {
        return Err(QueryError::Timestamp {
            value: end_date.to_rfc3339(),
        });
    };
    let promql = api_key.map_or_else(
        || GLOBAL_SPEND_QUERY.to_owned(),
        |key| {
            format!(
                "sum(delta(litellm_spend_metric_total{{hashed_api_key={}}}[1d]))",
                quote_promql_string_literal(key)
            )
        },
    );
    let params = vec![
        ("query", promql),
        ("start", isoformat(start_date)),
        ("end", isoformat(end_date)),
        ("step", DAY_STEP_SECONDS.to_owned()),
    ];
    let series_list = fetch_series(query, format!("{endpoint}/api/v1/query_range"), params).await?;
    daily_records(&series_list)
}

mod tests;
