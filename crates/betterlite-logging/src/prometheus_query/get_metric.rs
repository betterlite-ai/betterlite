use super::PrometheusQuery;
use super::clock::Clock;
use super::error::QueryError;
use super::fetch::fetch_series;
use super::response_records::PrometheusRangeSample;
use super::transport::Transport;

pub async fn get_metric_from_prometheus<Fetch, Time>(
    query: &PrometheusQuery<Fetch, Time>,
    metric_name: &str,
) -> Result<Vec<PrometheusRangeSample>, QueryError<Fetch::Error>>
where
    Fetch: Transport,
    Time: Clock,
{
    let endpoint = query.settings.endpoint()?;
    let params = vec![
        ("query", format!("{metric_name}[24h]")),
        ("time", query.clock.now().timestamp().to_string()),
    ];
    fetch_series(query, format!("{endpoint}/api/v1/query"), params).await
}

mod tests;
