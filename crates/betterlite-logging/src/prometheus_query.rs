pub mod clock;
pub mod daily_spend;
pub mod error;
pub mod fake_transport;
pub mod fallback_metric;
pub mod fetch;
pub mod get_metric;
pub mod is_connected;
pub mod promql_escaping;
pub mod response_records;
pub mod transport;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrometheusSettings {
    pub url: Option<String>,
    pub selected_instance: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PrometheusQuery<Fetch, Time> {
    pub settings: PrometheusSettings,
    pub transport: Fetch,
    pub clock: Time,
}
