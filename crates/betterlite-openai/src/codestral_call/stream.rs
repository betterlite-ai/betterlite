use super::error::CodestralCallError;
use super::log::{CallLogger, StreamPostCall};
use super::plan::Plan;
use crate::aleph_alpha::lines::{LineStream, decode_lines};
use crate::codestral_records::RecordSource;
use crate::codestral_records::chunk::{CodestralChunk, decode_codestral_chunk};
use crate::codestral_records::error::{NativeError, TextCompletionCodestralError};
use crate::codestral_records::record_text;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
const COMPLETION_OK: u16 = 200;
pub(super) async fn make_call(
    client: ProxiedClient,
    request: OutboundRequest,
    plan: Plan,
    record: RecordSource,
    mut log: Box<dyn CallLogger + Send>,
) -> Result<BoxStream<'static, Result<CodestralChunk, CodestralCallError>>, CodestralCallError> {
    let limit = client.limits.body_bytes;
    let response = client.send_following(request).await?;
    if response.response.status().as_u16() != COMPLETION_OK {
        return Err(status_failure(response, limit).await);
    }
    let logged = serde_json::to_string(&plan.body)
        .map_err(|source| CodestralCallError::Encode { source })?;
    log.stream_post_call(StreamPostCall {
        input: &plan.messages,
        api_key: "",
        original_response: "lines",
        request_data: &logged,
    });
    Ok(decoded(decode_lines(response, limit), record))
}
fn decoded(
    lines: LineStream,
    record: RecordSource,
) -> BoxStream<'static, Result<CodestralChunk, CodestralCallError>> {
    lines
        .map(move |line| match line {
            Ok(text) => {
                decode_codestral_chunk(text.as_bytes(), &record).map_err(CodestralCallError::Record)
            }
            Err(failure) => Err(CodestralCallError::Lines { source: failure }),
        })
        .boxed()
}
async fn status_failure(response: StreamResponse, limit: usize) -> CodestralCallError {
    match response.buffered(limit).await {
        Ok(failed) => {
            // make_call carries the decoded body text in its error, the same lossy text httpx's response.text holds
            let message = match record_text(&failed.body) {
                Ok(text) => text,
                Err(error) => return CodestralCallError::Record(error),
            };
            CodestralCallError::Record(TextCompletionCodestralError::Native(Box::new(
                NativeError::new(failed.status, message, None, None),
            )))
        }
        Err(failure) => CodestralCallError::Transport { source: failure },
    }
}
