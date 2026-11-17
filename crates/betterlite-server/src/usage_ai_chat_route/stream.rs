use std::convert::Infallible;
use std::future::pending;
use std::pin::pin;
use std::time::Duration;

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue};
use axum::response::Response;
use chrono::NaiveDate;
use futures_util::future::{BoxFuture, Either, select};
use futures_util::stream::{Stream, unfold};
use tokio::sync::mpsc;
use tokio::time::sleep;

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
    };
    unfold(state, next_frame)
}

async fn next_frame(mut state: FrameState) -> Option<(Result<String, Infallible>, FrameState)> {
    loop {
        // biased keeps LiteLLM's order: a frame that is ready wins over a ping
        let next = {
            let interval = state.interval;
            let mut frame = pin!(state.frames.recv());
            let event = select(frame.as_mut(), state.producer.as_mut());
            let mut ping = pin!(ping_due(interval));
            match select(event, ping.as_mut()).await {
                Either::Left((Either::Left((frame, _)), _)) => NextFrame::Frame(frame),
                Either::Left((Either::Right(((), _)), _)) => NextFrame::ProducerDone,
                Either::Right(((), _)) => NextFrame::Ping,
            }
        };
        match next {
            NextFrame::Frame(Some(frame)) => return Some((Ok(frame), state)),
            NextFrame::Frame(None) => return None,
            NextFrame::ProducerDone => state.producer = Box::pin(pending()),
            NextFrame::Ping => return Some((Ok(SSE_COMMENT_PING.to_owned()), state)),
        }
    }
}

async fn ping_due(interval: Option<Duration>) {
    match interval {
        Some(wait) => sleep(wait).await,
        None => pending::<()>().await,
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
