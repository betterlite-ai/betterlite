#[derive(Debug, thiserror::Error)]
pub enum QueryError<Failure> {
    #[error("PROMETHEUS_URL not set please set 'PROMETHEUS_URL=<>' in .env")]
    NotConfigured,
    #[error("the Prometheus request failed")]
    Transport(#[source] Failure),
    #[error("the Prometheus response has {length} bytes, over the limit of {limit}")]
    ResponseTooLarge { length: usize, limit: usize },
    #[error("the Prometheus response does not decode")]
    Decode(#[source] serde_json::Error),
    #[error("the Prometheus sample value {value:?} is not a finite number")]
    SampleValue { value: String },
    #[error("the timestamp {value} is outside the supported date range")]
    Timestamp { value: String },
}
