use std::convert::Infallible;
use std::time::Duration;

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue};
use axum::response::Response;
use chrono::NaiveDate;
use futures_util::future::BoxFuture;
use futures_util::stream::{Stream, unfold};
use tokio::sync::mpsc;

use crate::usage_ai_chat::flow::stream_usage_ai_chat;
use crate::usage_ai_chat::model::UsageChatModel;
use crate::usage_ai_chat::parts::{UsageChatParts, UsageChatRequest};
use crate::usage_ai_chat::source::UsageDataSource;

use super::body::ChatBody;

pub(super) const SSE_COMMENT_PING: &str = ": ping\n\n";
const FRAME_CHANNEL_CAPACITY: usize = 32;

struct FrameState {
    producer: BoxFuture<'static, ()>,
    frames: mpsc::Receiver<String>,
    interval: Option<Duration>,
    producer_done: bool,
}

enum NextFrame {
    Frame(Option<String>),
    ProducerDone,
    Ping,
}

pub(super) fn frame_stream<Model, Data, Clock>(
    parts: UsageChatParts<Model, Data, Clock>,
    body: ChatBody,
    user_id: Option<String>,
    is_admin: bool,
    interval: Option<Duration>,
) -> impl Stream<Item = Result<String, Infallible>> + Send + 'static
where
    Model: UsageChatModel + Send + 'static,
    Data: UsageDataSource + Send + 'static,
    Clock: Fn() -> NaiveDate + Send + Sync + 'static,
{
    let (sink, frames) = mpsc::channel(FRAME_CHANNEL_CAPACITY);
    let producer = async move {
        let request = UsageChatRequest {
            messages: &body.messages,
            model: body.model.as_deref(),
            user_id: user_id.as_deref(),
            is_admin,
        };
        stream_usage_ai_chat(&parts, &request, &sink).await;
    };
    let state = FrameState {
        producer: Box::pin(producer),
        frames,
        interval,
        producer_done: false,
    };
    unfold(state, next_frame)
}

async fn next_frame(mut state: FrameState) -> Option<(Result<String, Infallible>, FrameState)> {
    loop {
        let next = tokio::select! {
            frame = state.frames.recv() => NextFrame::Frame(frame),
            () = &mut state.producer, if !state.producer_done => NextFrame::ProducerDone,
            () = ping_due(state.interval) => NextFrame::Ping,
        };
        match next {
            NextFrame::Frame(Some(frame)) => return Some((Ok(frame), state)),
            NextFrame::Frame(None) => return None,
            NextFrame::ProducerDone => state.producer_done = true,
            NextFrame::Ping => return Some((Ok(SSE_COMMENT_PING.to_owned()), state)),
        }
    }
}

async fn ping_due(interval: Option<Duration>) {
    match interval {
        Some(wait) => tokio::time::sleep(wait).await,
        None => std::future::pending::<()>().await,
    }
}

pub(super) fn sse_response<Frames>(frames: Frames) -> Response
where
    Frames: Stream<Item = Result<String, Infallible>> + Send + 'static,
{
    let mut response = Response::new(Body::from_stream(frames));
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}
