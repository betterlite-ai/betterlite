use std::error::Error;
use std::mem::replace;

use async_trait::async_trait;
use futures_util::future::{Either, select};

use super::{Drain, HookState, MessagesAgenticStream};
use crate::api::messages_agentic_stream::interceptor::AgenticInterceptor;
use crate::api::messages_agentic_stream::{
    KEEPALIVE_PING, MessagesAgenticError, MessagesAgenticStreamError,
};
use crate::api::messages_sse_events::stream::MessagesProviderStream;
use crate::meta_muse_realtime::request::Sleep;

// the workspace denies multiple_inherent_impl, so the hold-back read path lives on this trait
#[async_trait]
pub(super) trait HeldBack {
    type Failure;

    async fn read_held_back(&mut self) -> Result<Option<Vec<u8>>, Self::Failure>;

    async fn drain_upstream(&mut self, interval: f64) -> Result<bool, Self::Failure>;

    async fn hook_completed(&mut self, interval: f64) -> bool;

    async fn run_hooks(&mut self);
}

#[async_trait]
impl<S, I> HeldBack for MessagesAgenticStream<S, I>
where
    S: MessagesProviderStream + Send,
    I: AgenticInterceptor,
    S::Error: Error + 'static,
    <I::FollowUp as MessagesProviderStream>::Error: Error + 'static,
{
    type Failure = MessagesAgenticStreamError<S, I>;

    // LiteLLM's _anext_held_back: a ping whenever no owned read finished within the interval
    async fn read_held_back(&mut self) -> Result<Option<Vec<u8>>, Self::Failure> {
        let interval = self.ping_interval();
        match self.drain {
            Drain::Idle => {
                self.drain = Drain::Draining;
                return Ok(Some(KEEPALIVE_PING.to_vec()));
            }
            Drain::Draining => {
                if !self.drain_upstream(interval).await? {
                    return Ok(Some(KEEPALIVE_PING.to_vec()));
                }
                self.drain = Drain::Exhausted;
            }
            Drain::Exhausted => {}
        }
        if !self.hook_completed(interval).await {
            return Ok(Some(KEEPALIVE_PING.to_vec()));
        }
        if let Some(follow_up) = self.follow_up.as_mut() {
            return next_follow_up(follow_up, &self.clock, interval)
                .await
                .map_err(MessagesAgenticError::FollowUp);
        }
        if self.server_tool_holds()? {
            return Ok(self.leak_verdict());
        }
        Ok(self.replay_next())
    }

    // LiteLLM's _drain_upstream, given one interval per client read; true means the stream ended
    async fn drain_upstream(&mut self, interval: f64) -> Result<bool, Self::Failure> {
        let mut deadline = (self.clock)(interval);
        loop {
            let read = match select(self.inner.next_chunk(), deadline).await {
                Either::Left((finished, pending)) => {
                    deadline = pending;
                    finished
                }
                Either::Right(((), _unfinished)) => return Ok(false),
            };
            let Some(chunk) = read.map_err(MessagesAgenticError::Provider)? else {
                return Ok(true);
            };
            self.collect(chunk)?;
        }
    }

    // LiteLLM's _completed_within_ping_interval around the hook task
    async fn hook_completed(&mut self, interval: f64) -> bool {
        if matches!(self.hooks, HookState::NotStarted) {
            let Some(started) = self.hook_future() else {
                return true;
            };
            self.hooks = HookState::Running(started);
        }
        let HookState::Running(running) = replace(&mut self.hooks, HookState::Done) else {
            return true;
        };
        let sleep = (self.clock)(interval);
        match select(running, sleep).await {
            Either::Left((outcome, _sleep)) => {
                self.finish_hooks(outcome);
                true
            }
            Either::Right(((), pending)) => {
                self.hooks = HookState::Running(pending);
                false
            }
        }
    }

    // LiteLLM's _process_agentic_hooks awaited inline by the live __anext__ path
    async fn run_hooks(&mut self) {
        let Some(hooks) = self.hook_future() else {
            return;
        };
        let outcome = hooks.await;
        self.finish_hooks(outcome);
    }
}

// LiteLLM's _next_follow_up_chunk: one read per client read, a ping when it does not finish
async fn next_follow_up<F>(
    follow_up: &mut F,
    clock: &Sleep,
    interval: f64,
) -> Result<Option<Vec<u8>>, F::Error>
where
    F: MessagesProviderStream + Send,
{
    match select(follow_up.next_chunk(), clock(interval)).await {
        Either::Left((chunk, _sleep)) => chunk,
        Either::Right(((), _unfinished)) => Ok(Some(KEEPALIVE_PING.to_vec())),
    }
}
