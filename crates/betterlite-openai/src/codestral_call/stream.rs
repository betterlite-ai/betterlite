use std::mem;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use futures_util::Stream;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, unfold};

use super::error::CodestralCallError;
use super::log::{CallLogger, OriginalResponse, StreamPostCall};
use super::plan::Plan;
use crate::codestral_records::RecordSource;
use crate::codestral_records::chunk::{CodestralChunk, decode_codestral_chunk};
use crate::codestral_records::error::{NativeError, TextCompletionCodestralError};
use crate::codestral_records::record_text;

pub(super) fn open(
    client: ProxiedClient,
    request: OutboundRequest,
    plan: Plan,
    source: RecordSource,
    log: Box<dyn CallLogger + Send>,
) -> impl Stream<Item = Result<CodestralChunk, CodestralCallError>> + Send {
    let limit = client.limits.body_bytes;
    unfold(
        Phase::Cold(Box::new(Cold {
            client,
            request,
            plan,
            source,
            log,
            limit,
        })),
        next_chunk,
    )
}

struct Cold {
    client: ProxiedClient,
    request: OutboundRequest,
    plan: Plan,
    source: RecordSource,
    log: Box<dyn CallLogger + Send>,
    limit: usize,
}

enum Phase {
    Cold(Box<Cold>),
    Lines {
        lines: ByteLines,
        source: RecordSource,
    },
    Done,
}

async fn next_chunk(phase: Phase) -> Option<(Result<CodestralChunk, CodestralCallError>, Phase)> {
    match phase {
        Phase::Cold(cold) => cold_step(*cold).await,
        Phase::Lines { mut lines, source } => {
            let item = next_item(&mut lines, &source).await?;
            Some((item, Phase::Lines { lines, source }))
        }
        Phase::Done => None,
    }
}

async fn cold_step(cold: Cold) -> Option<(Result<CodestralChunk, CodestralCallError>, Phase)> {
    let Cold {
        client,
        request,
        plan,
        source,
        mut log,
        limit,
    } = cold;
    let response = match client.send_following(request).await {
        Ok(response) => response,
        Err(failure) => {
            return Some((
                Err(CodestralCallError::Transport { source: failure }),
                Phase::Done,
            ));
        }
    };
    if response.response.status().as_u16() != 200 {
        return Some((Err(status_failure(response, limit).await), Phase::Done));
    }
    let logged = match serde_json::to_string(&plan.body) {
        Ok(text) => text,
        Err(failure) => {
            return Some((
                Err(CodestralCallError::Encode { source: failure }),
                Phase::Done,
            ));
        }
    };
    log.stream_post_call(StreamPostCall {
        input: &plan.messages,
        api_key: "",
        original_response: OriginalResponse::Lines,
        request_data: &logged,
    });
    let mut lines = ByteLines::new(response.body_stream(), limit);
    let item = next_item(&mut lines, &source).await?;
    Some((item, Phase::Lines { lines, source }))
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

async fn next_item(
    lines: &mut ByteLines,
    source: &RecordSource,
) -> Option<Result<CodestralChunk, CodestralCallError>> {
    let next = lines.next().await?;
    Some(match next {
        Ok(line) => decode_codestral_chunk(&line, source).map_err(CodestralCallError::Record),
        Err(error) => Err(error),
    })
}

struct ByteLines {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pending: Vec<u8>,
    limit: usize,
    finished: bool,
}

impl ByteLines {
    fn new(body: BoxStream<'static, Result<Vec<u8>, HttpError>>, limit: usize) -> Self {
        Self {
            body,
            pending: Vec::new(),
            limit,
            finished: false,
        }
    }

    async fn next(&mut self) -> Option<Result<Vec<u8>, CodestralCallError>> {
        loop {
            if let Some(line) = self.ready_line() {
                return Some(line);
            }
            if self.pending.len() > self.limit {
                return Some(self.fail(CodestralCallError::LineTooLong { limit: self.limit }));
            }
            if self.finished {
                return self.drain_tail();
            }
            match self.body.next().await {
                Some(Ok(chunk)) => self.pending.extend_from_slice(&chunk),
                Some(Err(source)) => return Some(self.reject(source)),
                None => self.finished = true,
            }
        }
    }

    fn ready_line(&mut self) -> Option<Result<Vec<u8>, CodestralCallError>> {
        let position = self.pending.iter().position(|byte| *byte == b'\n');
        match position {
            Some(index) if index > self.limit => {
                let limit = self.limit;
                Some(self.fail(CodestralCallError::LineTooLong { limit }))
            }
            Some(index) => Some(Ok(take_line(&mut self.pending, index))),
            None => None,
        }
    }

    fn drain_tail(&mut self) -> Option<Result<Vec<u8>, CodestralCallError>> {
        if self.pending.is_empty() {
            return None;
        }
        Some(Ok(mem::take(&mut self.pending)))
    }

    fn reject(&mut self, source: HttpError) -> Result<Vec<u8>, CodestralCallError> {
        self.fail(CodestralCallError::Transport {
            source: TransportError::Rejected(source),
        })
    }

    fn fail(&mut self, error: CodestralCallError) -> Result<Vec<u8>, CodestralCallError> {
        self.pending.clear();
        self.finished = true;
        Err(error)
    }
}

fn take_line(pending: &mut Vec<u8>, index: usize) -> Vec<u8> {
    let tail = pending.split_off(index.saturating_add(1));
    let mut line = mem::replace(pending, tail);
    line.truncate(line.len().saturating_sub(1));
    if line.last() == Some(&b'\r') {
        line.truncate(line.len().saturating_sub(1));
    }
    line
}
