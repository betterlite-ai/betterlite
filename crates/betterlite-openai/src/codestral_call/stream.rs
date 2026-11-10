use super::error::CodestralCallError;
use super::log::{CallLogger, StreamPostCall};
use super::plan::{Mode, Plan};
use crate::aleph_alpha::lines::{LineStream, decode_lines};
use crate::codestral_records::RecordSource;
use crate::codestral_records::chunk::{CodestralChunk, decode_codestral_chunk as decode};
use crate::codestral_records::error::{NativeError, TextCompletionCodestralError};
use crate::codestral_records::record_text;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
pub type ChunkStream = BoxStream<'static, Result<CodestralChunk, CodestralCallError>>;
const COMPLETION_OK: u16 = 200;
pub(super) async fn make_call(
    client: ProxiedClient,
    request: OutboundRequest,
    plan: Plan,
    record: RecordSource,
    mut log: Box<dyn CallLogger + Send>,
) -> Result<ChunkStream, CodestralCallError> {
    let limit = client.limits.body_bytes;
    let timed = |source| CodestralCallError::transport(source, false);
    let response = client.send_following(request).await.map_err(timed)?;
    if response.response.status().as_u16() != COMPLETION_OK {
        return Err(status_failure(response, limit).await);
    }
    if plan.mode == Mode::Async {
        let logged = serde_json::to_string(&plan.body)
            .map_err(|source| CodestralCallError::Encode { source })?;
        log.stream_post_call(StreamPostCall {
            input: &plan.messages,
            api_key: "",
            original_response: "lines",
            request_data: &logged,
        });
    }
    Ok(decoded(decode_lines(response, limit), record))
}
fn decoded(lines: LineStream, record: RecordSource) -> ChunkStream {
    lines
        .map(move |line| match line {
            Ok(text) => decode(text.as_bytes(), &record).map_err(CodestralCallError::Record),
            Err(failure) => Err(CodestralCallError::Lines { source: failure }),
        })
        .boxed()
}
pub(super) fn native_failure(
    status: u16,
    body: &[u8],
) -> Result<CodestralCallError, CodestralCallError> {
    let message = record_text(body)?;
    Ok(CodestralCallError::Record(
        TextCompletionCodestralError::Native(Box::new(NativeError::new(
            status, message, None, None,
        ))),
    ))
}
async fn status_failure(response: StreamResponse, limit: usize) -> CodestralCallError {
    match response.buffered(limit).await {
        Ok(failed) => match native_failure(failed.status, &failed.body) {
            Ok(error) | Err(error) => error,
        },
        Err(failure) => CodestralCallError::transport(failure, false),
    }
}
