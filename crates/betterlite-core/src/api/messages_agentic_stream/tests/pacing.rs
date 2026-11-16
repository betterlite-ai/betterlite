#![cfg(test)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use tokio::time::sleep;

use super::{
    DelayLog, EffectLog, HookReply, HookSeen, ScriptedInner, agentic_input, drain, policy,
    policy_at, recorded_sleep, replacement_frames, resolved_interceptor, server_tool_stream,
    text_stream,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_agentic_stream::stream::MessagesAgenticStream;
use crate::api::messages_agentic_stream::{HoldBackPolicy, KEEPALIVE_PING, MessagesAgenticError};
use crate::api::messages_sse_events::stream::MessagesProviderStream;
use crate::meta_muse_realtime::request::Sleep;

// a provider that answers every read after the same delay and never fails
struct PacedInner {
    frames: VecDeque<Vec<u8>>,
    pace: Duration,
    effects: EffectLog,
}

#[async_trait]
impl MessagesProviderStream for PacedInner {
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        sleep(self.pace).await;
        let frame = self.frames.pop_front();
        self.effects.record(if frame.is_some() {
            "inner-read"
        } else {
            "inner-end"
        });
        Ok(frame)
    }
}

fn tokio_sleep() -> Sleep {
    Arc::new(|seconds: f64| -> BoxFuture<'static, ()> {
        Box::pin(sleep(
            Duration::try_from_secs_f64(seconds).unwrap_or_default(),
        ))
    })
}

// a provider that sends faster than the interval still yields a ping once the interval passes, as LiteLLM's wait_for bounds each read
#[tokio::test(start_paused = true)]
async fn drain_pings_within_interval() {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let frames = text_stream();
    let inner = PacedInner {
        frames: VecDeque::from(frames.clone()),
        pace: Duration::from_millis(10),
        effects: effects.clone(),
    };
    let mut stream = MessagesAgenticStream::new(
        inner,
        resolved_interceptor(HookReply::Empty, &effects, &seen),
        agentic_input(Some(policy_at(&[], 0.025_f64))),
        tokio_sleep(),
        ApiLimits::default(),
    );
    let first = stream.next_chunk().await.expect("the first held-back read");
    assert_eq!(first, Some(KEEPALIVE_PING.to_vec()));
    let second = stream.next_chunk().await.expect("the draining read");
    assert_eq!(second, Some(KEEPALIVE_PING.to_vec()));
    assert_eq!(effects.count("inner-read"), 2);
    assert_eq!(effects.count("inner-end"), 0);
    let replayed = drain(&mut stream)
        .await
        .into_iter()
        .filter(|chunk| chunk.as_slice() != KEEPALIVE_PING)
        .collect::<Vec<_>>();
    assert_eq!(replayed, frames);
    assert_eq!(effects.count("inner-end"), 1);
}

// a follow-up read that misses the interval yields a ping, and the chunk arrives on a later read
#[tokio::test]
async fn follow_up_pings_while_pending() {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let delays = DelayLog::default();
    let gate = Arc::new(AtomicBool::new(false));
    let reply = HookReply::Stream(replacement_frames(), Some(Arc::clone(&gate)));
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(server_tool_stream("web_search"), &effects),
        resolved_interceptor(reply, &effects, &seen),
        agentic_input(Some(policy_at(&["web_search"], 0.05_f64))),
        recorded_sleep(&delays),
        ApiLimits::default(),
    );
    let first = stream.next_chunk().await.expect("the first held-back read");
    assert_eq!(first, Some(KEEPALIVE_PING.to_vec()));
    let pending = stream
        .next_chunk()
        .await
        .expect("the pending follow-up read");
    assert_eq!(pending, Some(KEEPALIVE_PING.to_vec()));
    assert_eq!(effects.count("hook-start"), 1);
    assert_eq!(effects.count("followup-read"), 0);
    gate.store(true, Ordering::SeqCst);
    let ready = stream
        .next_chunk()
        .await
        .expect("the released follow-up read");
    assert_eq!(ready, replacement_frames().first().cloned());
    assert_eq!(effects.count("followup-read"), 1);
    assert!(
        delays
            .snapshot()
            .iter()
            .all(|bits| *bits == 0.05_f64.to_bits())
    );
}

// LiteLLM's HOLD_BACK_PING_INTERVAL_SECONDS and its empty frozenset of server-fulfilled names
#[test]
fn default_policy_matches_litellm() {
    let policy = HoldBackPolicy::default();
    assert_eq!(policy.ping_interval_seconds.to_bits(), 15.0_f64.to_bits());
    assert!(policy.server_fulfilled_tool_names.is_empty());
}

// collected bytes that are not UTF-8 skip the hooks and fail the leak check instead of being replaced
#[tokio::test]
async fn invalid_utf8_fails_leak_check() {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let mut frames = server_tool_stream("web_search");
    frames.push(vec![0xff]);
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(frames, &effects),
        resolved_interceptor(HookReply::Empty, &effects, &seen),
        agentic_input(Some(policy(&["web_search"]))),
        recorded_sleep(&DelayLog::default()),
        ApiLimits::default(),
    );
    let first = stream.next_chunk().await.expect("the first held-back read");
    assert_eq!(first, Some(KEEPALIVE_PING.to_vec()));
    let failed = stream.next_chunk().await;
    assert!(matches!(failed, Err(MessagesAgenticError::Utf8(_))));
    assert_eq!(effects.count("hook-start"), 0);
}
