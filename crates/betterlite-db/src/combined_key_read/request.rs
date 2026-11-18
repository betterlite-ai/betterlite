use opentelemetry::trace::SpanContext;
use secrecy::SecretString;

pub const COMBINED_VIEW_TABLE: &str = "combined_view";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyQueryType {
    FindUnique,
    FindAll,
}

// the token is a secret: Debug prints it redacted, and every clone keeps the value out of logs
#[derive(Debug, Clone)]
pub struct CombinedKeyRequest {
    pub token: Option<SecretString>,
    pub query_type: KeyQueryType,
    pub check_deprecated: bool,
    pub use_writer: bool,
    pub parent_otel_span: Option<SpanContext>,
}
