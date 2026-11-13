pub const US_METRIC_ENDPOINT: &str = "https://metric-api.newrelic.com/metric/v1";
pub const EU_METRIC_ENDPOINT: &str = "https://metric-api.eu.newrelic.com/metric/v1";
pub const DEFAULT_REGION: &str = "us";

// A fixed table by design: team config picks a region, so callback vars can never redirect metrics to an arbitrary host
const METRIC_ENDPOINT_BY_REGION: [(&str, &str); 2] =
    [("us", US_METRIC_ENDPOINT), ("eu", EU_METRIC_ENDPOINT)];

#[must_use]
pub fn resolve_metric_endpoint(region: Option<&str>) -> &'static str {
    let Some(selected) = region.filter(|value| !value.is_empty()) else {
        return US_METRIC_ENDPOINT;
    };
    let lowered = selected.to_lowercase();
    METRIC_ENDPOINT_BY_REGION
        .iter()
        .find(|entry| entry.0 == lowered.as_str())
        .map_or_else(
            || {
                let mut supported: Vec<&str> = METRIC_ENDPOINT_BY_REGION
                    .iter()
                    .map(|entry| entry.0)
                    .collect();
                supported.sort_unstable();
                tracing::warn!(
                    region = selected,
                    supported = supported.join(", "),
                    "New Relic: unknown newrelic_region, using the US Metric API endpoint"
                );
                US_METRIC_ENDPOINT
            },
            |entry| entry.1,
        )
}
