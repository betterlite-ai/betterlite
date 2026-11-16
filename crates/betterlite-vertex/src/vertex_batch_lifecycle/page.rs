use std::str::from_utf8;

use betterlite_http::proxy::UpstreamResponse;

use crate::vertex_batch_create::payload::decode_vertex_batch_payload;
use crate::vertex_batch_records::types::VertexBatchJobPage;

use super::error::VertexBatchLifecycleError;

mod tests;

// the list call checks for exactly 200 before it reads the page, and the page body takes the create path's bound
pub fn decode_vertex_batch_page(
    reply: &UpstreamResponse,
) -> Result<VertexBatchJobPage, VertexBatchLifecycleError> {
    if reply.status != 200 {
        return Err(VertexBatchLifecycleError::ListStatus {
            status: reply.status,
            message: body_text(&reply.body),
        });
    }
    let page: VertexBatchJobPage = decode_vertex_batch_payload(&reply.body)?;
    Ok(page)
}

// httpx decodes response.text leniently, so a body that is not UTF-8 keeps the text before its first invalid byte
pub(crate) fn body_text(body: &[u8]) -> String {
    match from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(invalid) => from_utf8(body.get(..invalid.valid_up_to()).unwrap_or_default())
            .unwrap_or_default()
            .to_owned(),
    }
}
