use super::PrometheusQuery;
use super::clock::Clock;
use super::error::QueryError;
use super::response_records::{PrometheusQueryResponse, PrometheusRangeSample};
use super::transport::{MAX_RESPONSE_BYTES, QUERY_TIMEOUT, QueryRequest, Transport};

pub(super) async fn fetch_series<Fetch, Time>(
    query: &PrometheusQuery<Fetch, Time>,
    url: String,
    params: Vec<(&'static str, String)>,
) -> Result<Vec<PrometheusRangeSample>, QueryError<Fetch::Error>>
where
    Fetch: Transport,
    Time: Clock,
{
    let request = QueryRequest {
        url,
        params,
        timeout: QUERY_TIMEOUT,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    let body = query
        .transport
        .get(request)
        .await
        .map_err(QueryError::Transport)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(QueryError::ResponseTooLarge {
            length: body.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let response: PrometheusQueryResponse =
        serde_json::from_slice(&body).map_err(QueryError::Decode)?;
    Ok(response.data.result)
}
