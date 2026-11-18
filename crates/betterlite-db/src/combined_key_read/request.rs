use opentelemetry::trace::SpanContext;

pub const COMBINED_VIEW_TABLE: &str = "combined_view";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyQueryType {
    FindUnique,
    FindAll,
}

impl KeyQueryType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FindUnique => "find_unique",
            Self::FindAll => "find_all",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CombinedKeyRequest<'request> {
    pub token: Option<&'request str>,
    pub query_type: KeyQueryType,
    pub check_deprecated: bool,
    pub use_writer: bool,
    pub parent_otel_span: Option<SpanContext>,
}

impl<'request> CombinedKeyRequest<'request> {
    #[must_use]
    pub fn find_unique(token: Option<&'request str>) -> Self {
        Self {
            token,
            query_type: KeyQueryType::FindUnique,
            check_deprecated: true,
            use_writer: false,
            parent_otel_span: None,
        }
    }
}
