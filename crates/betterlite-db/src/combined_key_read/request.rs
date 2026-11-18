use opentelemetry::trace::SpanContext;

pub const COMBINED_VIEW_TABLE: &str = "combined_view";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyQueryType {
    FindUnique,
    FindAll,
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

    // the deprecated key recurses once with check_deprecated off, so a grace lookup can never chain
    #[must_use]
    pub fn deprecated_hop<'token>(
        &self,
        active_token_id: &'token str,
    ) -> CombinedKeyRequest<'token> {
        CombinedKeyRequest {
            token: Some(active_token_id),
            query_type: KeyQueryType::FindUnique,
            check_deprecated: false,
            use_writer: self.use_writer,
            parent_otel_span: self.parent_otel_span.clone(),
        }
    }
}
