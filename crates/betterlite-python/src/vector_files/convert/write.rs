use betterlite_core::api::vector_file_write::{FileUpdateRequest, FileWriteRequest};

use super::WriteWire;

// the write both file writes share
pub(crate) fn write_request(wire: &WriteWire) -> FileWriteRequest<'_> {
    FileWriteRequest {
        vector_store_id: &wire.vector_store_id,
        file_id: &wire.file_id,
        extra_headers: wire.extra_headers.as_ref(),
        timeout: wire.timeout,
        custom_llm_provider: wire.custom_llm_provider.as_deref(),
        kwargs: &wire.kwargs,
    }
}

// update's own arguments: the shared write plus the attributes and body its handler restores
pub(crate) fn update_request(wire: &WriteWire) -> FileUpdateRequest<'_> {
    FileUpdateRequest {
        write: write_request(wire),
        attributes: wire.attributes.as_ref(),
        extra_body: wire.extra_body.as_ref(),
    }
}
