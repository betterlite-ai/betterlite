use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::Future;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;

use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_core::api::messages_agentic_stream::MessagesAgenticStreamInput;
use betterlite_core::api::messages_agentic_stream::MessagesAgenticStreamResult;
use betterlite_core::api::messages_agentic_stream::interceptor::AgenticFollowUp;
use betterlite_core::api::messages_agentic_stream::interceptor::AgenticHookFuture;
use betterlite_core::api::messages_agentic_stream::interceptor::AgenticInterceptor;
use betterlite_core::api::messages_fake_stream::MessagesFakeStream;
use betterlite_core::api::messages_fake_stream::MessagesFakeStreamError;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_models::decisions::Json;
use serde_json::Value;
use serde_json::json;

use super::Reply;
use super::ReplyKind;

pub(crate) struct Shared {
    pub(crate) drain_gate: Arc<AtomicBool>,
    pub(crate) hook_gate: Arc<AtomicBool>,
    pub(crate) effects: Arc<Mutex<Vec<String>>>,
    pub(crate) hook_calls: Arc<Mutex<Vec<Value>>>,
}

pub(crate) struct ScriptedInner {
    pub(crate) frames: VecDeque<Vec<u8>>,
    pub(crate) gate: Arc<AtomicBool>,
    pub(crate) effects: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl MessagesProviderStream for ScriptedInner {
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        wait_until_open(&self.gate).await;
        Ok(self.frames.pop_front())
    }

    async fn aclose(&mut self) -> Option<Result<(), Infallible>> {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("inner-close".to_owned());
        Some(Ok(()))
    }
}

pub(crate) struct ScriptedFollowUp {
    pub(crate) frames: VecDeque<Vec<u8>>,
    pub(crate) gate: Arc<AtomicBool>,
    pub(crate) effects: Option<Arc<Mutex<Vec<String>>>>,
}

#[async_trait]
impl MessagesProviderStream for ScriptedFollowUp {
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        wait_until_open(&self.gate).await;
        Ok(self.frames.pop_front())
    }

    async fn aclose(&mut self) -> Option<Result<(), Infallible>> {
        let effects = self.effects.as_ref()?;
        effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("followup-close".to_owned());
        Some(Ok(()))
    }
}

impl AgenticFollowUp for ScriptedFollowUp {
    fn from_response(response: &Json) -> Result<Self, MessagesFakeStreamError> {
        Ok(Self {
            frames: MessagesFakeStream::new(response)?.collect(),
            gate: Arc::new(AtomicBool::new(true)),
            effects: None,
        })
    }
}

pub(crate) struct ScriptedInterceptor {
    pub(crate) reply: Mutex<Option<Reply>>,
    pub(crate) gate: Arc<AtomicBool>,
    pub(crate) hook_calls: Arc<Mutex<Vec<Value>>>,
    pub(crate) effects: Arc<Mutex<Vec<String>>>,
}

impl AgenticInterceptor for ScriptedInterceptor {
    type FollowUp = ScriptedFollowUp;

    fn intercept(
        &self,
        response: &Json,
        input: &MessagesAgenticStreamInput,
    ) -> AgenticHookFuture<Self::FollowUp> {
        self.hook_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"model": input.model, "response": response}));
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let outcome = match reply {
            Some(reply) => match reply.kind {
                ReplyKind::Absent => Ok(MessagesAgenticStreamResult::NoFollowUp),
                ReplyKind::Stream => Ok(MessagesAgenticStreamResult::Stream(ScriptedFollowUp {
                    frames: reply
                        .chunks
                        .iter()
                        .map(|chunk| chunk.as_bytes().to_vec())
                        .collect(),
                    gate: Arc::new(AtomicBool::new(true)),
                    effects: Some(Arc::clone(&self.effects)),
                })),
            },
            None => Err(AgenticHookError::Failed {
                callback: "messages_agentic_stream".to_owned(),
                message: "the scripted replies are used up".to_owned(),
            }),
        };
        Box::pin(GatedHook {
            gate: Arc::clone(&self.gate),
            outcome: Some(outcome),
            effects: Arc::clone(&self.effects),
        })
    }
}

pub(crate) struct GatedHook {
    gate: Arc<AtomicBool>,
    outcome: Option<Result<MessagesAgenticStreamResult<ScriptedFollowUp>, AgenticHookError>>,
    effects: Arc<Mutex<Vec<String>>>,
}

impl Future for GatedHook {
    type Output = Result<MessagesAgenticStreamResult<ScriptedFollowUp>, AgenticHookError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.gate.load(Ordering::SeqCst) {
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        match this.outcome.take() {
            Some(outcome) => Poll::Ready(outcome),
            None => Poll::Pending,
        }
    }
}

impl Drop for GatedHook {
    fn drop(&mut self) {
        if self.outcome.is_some() {
            self.effects
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push("hook-cancelled".to_owned());
        }
    }
}

async fn wait_until_open(gate: &Arc<AtomicBool>) {
    poll_fn(|context: &mut Context<'_>| {
        if gate.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}
