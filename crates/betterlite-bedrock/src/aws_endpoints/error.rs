#[derive(Debug, thiserror::Error)]
pub enum AwsEndpointError {
    #[error("invalid AWS region format: {region}")]
    Region { region: String },
    #[error("invalid AWS endpoint URL")]
    Endpoint {
        #[source]
        source: url::ParseError,
    },
    #[error("invalid field {field}")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}
