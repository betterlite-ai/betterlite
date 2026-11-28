use betterlite_core::api::vector_file_create::{ListFileRequest, TransportRequest};

use super::ListWire;

// the call B1072's list reads: a list handler takes no body, so its extra_body stays absent
pub(crate) fn list_request(wire: &ListWire) -> ListFileRequest<'_> {
    ListFileRequest {
        transport: TransportRequest {
            vector_store_id: &wire.vector_store_id,
            extra_headers: wire.extra_headers.as_ref(),
            extra_query: wire.extra_query.as_ref(),
            extra_body: None,
            timeout: wire.timeout,
            custom_llm_provider: wire.custom_llm_provider.as_deref(),
            kwargs: &wire.kwargs,
        },
        after: wire.after.as_deref(),
        before: wire.before.as_deref(),
        filter: wire.filter.as_deref(),
        limit: wire.limit,
        order: wire.order.as_deref(),
    }
}
