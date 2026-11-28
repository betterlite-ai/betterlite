use betterlite_core::api::vector_file_read::service::ReadCall;

use super::ReadWire;

// the call B1073's read session takes
pub(crate) fn read_call(wire: &ReadWire) -> ReadCall<'_> {
    ReadCall {
        vector_store_id: &wire.vector_store_id,
        file_id: &wire.file_id,
        extra_headers: wire.extra_headers.as_ref(),
        timeout: wire.timeout,
        custom_llm_provider: wire.custom_llm_provider.as_deref(),
        kwargs: &wire.kwargs,
    }
}
