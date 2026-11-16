use std::error::Error;
use std::mem::replace;

use futures_util::future::{Either, select};

use super::{HookState, MessagesAgenticStream};
use crate::api::messages_agentic_stream::interceptor::AgenticInterceptor;
use crate::api::messages_agentic_stream::{
    KEEPALIVE_PING, MessagesAgenticError, MessagesAgenticStreamError,
};
use crate::api::messages_sse_events::stream::MessagesProviderStream;

impl<S, I> MessagesAgenticStream<S, I>
where
    S: MessagesProviderStream + Send,
    I: AgenticInterceptor,
    S::Error: Error + 'static,
    <I::FollowUp as MessagesProviderStream>::Error: Error + 'static,
{
    // LiteLLM's _anext_held_back: a ping whenever no owned read finished within the interval
    pub(super) async fn read_held_back(
        &mut self,
    ) -> Result<Option<Vec<u8>>, MessagesAgenticStreamError<S, I>> {
        let interval = self.ping_interval();
        if !self.drain_started {
            self.drain_started = true;
            return Ok(Some(KEEPALIVE_PING.to_vec()));
        }
        if !self.stream_exhausted {
            if !self.drain_upstream(interval).await? {
                return Ok(Some(KEEPALIVE_PING.to_vec()));
            }
            self.stream_exhausted = true;
        }
        if !self.hook_completed(interval).await {
            return Ok(Some(KEEPALIVE_PING.to_vec()));
        }
        if self.follow_up.is_some() {
            return self.next_follow_up(interval).await;
        }
        if self.server_tool_holds()? {
            return Ok(self.leak_verdict());
        }
        Ok(self.replay_next())
    }

    // LiteLLM's _drain_upstream, given one interval per client read; true means the stream ended
    async fn drain_upstream(
        &mut self,
        interval: f64,
    ) -> Result<bool, MessagesAgenticStreamError<S, I>> {
        let mut deadline = (self.clock)(interval);
        loop {
            let read = match select(self.inner.next_chunk(), deadline).await {
                Either::Left((read, pending)) => {
                    deadline = pending;
                    read
                }
                Either::Right(((), _read)) => return Ok(false),
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
            let Some(future) = self.hook_future() else {
                self.hooks = HookState::Done;
                return true;
            };
            self.hooks = HookState::Running(future);
        }
        let HookState::Running(future) = replace(&mut self.hooks, HookState::Done) else {
            return true;
        };
        let sleep = (self.clock)(interval);
        let outcome = select(future, sleep).await;
        match outcome {
            Either::Left((outcome, _sleep)) => {
                self.finish_hooks(outcome);
                true
            }
            Either::Right(((), future)) => {
                self.hooks = HookState::Running(future);
                false
            }
        }
    }

    // LiteLLM's _process_agentic_hooks awaited inline by the live __anext__ path
    pub(super) async fn run_hooks(&mut self) {
        let Some(future) = self.hook_future() else {
            return;
        };
        let outcome = future.await;
        self.finish_hooks(outcome);
    }

    // LiteLLM's _next_follow_up_chunk: one read per client read, a ping when it does not finish
    async fn next_follow_up(
        &mut self,
        interval: f64,
    ) -> Result<Option<Vec<u8>>, MessagesAgenticStreamError<S, I>> {
        let Some(follow_up) = self.follow_up.as_mut() else {
            return Ok(None);
        };
        let sleep = (self.clock)(interval);
        let outcome = select(follow_up.next_chunk(), sleep).await;
        match outcome {
            Either::Left((chunk, _sleep)) => chunk.map_err(MessagesAgenticError::FollowUp),
            Either::Right(((), _read)) => Ok(Some(KEEPALIVE_PING.to_vec())),
        }
    }
}
