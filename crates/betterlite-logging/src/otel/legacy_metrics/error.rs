use crate::private_json::error::PrivateJsonError;

#[derive(Debug, thiserror::Error)]
pub enum LegacyMetricsError {
    #[error("otel.attributes must be a mapping with optional 'include_list' / 'exclude_list'")]
    AttributesNotMapping,
    #[error("otel.attributes: {source}")]
    AttributesShape {
        #[source]
        source: serde_json::Error,
    },
    #[error("otel.attributes: include_list and exclude_list are mutually exclusive")]
    BothLists,
    #[error(
        "otel.attributes: gen_ai.token.type is a structural token-usage discriminator and cannot be filtered"
    )]
    TokenTypeFiltered,
    #[error("otel.attributes: unknown attribute name(s) {names:?}")]
    UnknownAttributeNames { names: Vec<String> },
    #[error("{field} has the wrong type, expected {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the call times are out of range")]
    TimeRange,
    #[error("cannot serialize a metric attribute: {source}")]
    Dump {
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot serialize event content: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
