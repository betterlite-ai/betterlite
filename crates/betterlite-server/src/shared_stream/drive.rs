use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use futures_util::StreamExt as _;

use super::call::{SharedCall, SharedSource, StreamAbort};
use super::failure::finish_failure;
use super::framing::ChunkFraming;
use super::ports::{CostInjection, ReservationRelease};
use super::state::Guard;
use super::tail::ANTHROPIC_PING_BYTES;
use crate::sse_data_generator::types::{
    CleanupOutcome, ItemStream, StreamCleanup, StreamFailure, StreamHooks,
};
use crate::stream_chunks::ChunkKind;
use crate::stream_chunks::StreamItem;
use crate::stream_chunks::serialize::chunk_response_text;

pub(super) struct Ports<Key, H> {
    pub(super) hooks: Arc<H>,
    pub(super) reservation: Arc<dyn ReservationRelease<Key>>,
    pub(super) cleanup: Arc<dyn StreamCleanup<Key>>,
    pub(super) cost: Option<Arc<dyn CostInjection>>,
    pub(super) redactor: Option<Arc<Redactor>>,
}

pub(super) enum Step<Out> {
    Write(Out),
    End(Option<Out>),
    Failed(Result<Out, StreamAbort>),
}

pub(super) struct Drive<Key, H, F> {
    hooks: Arc<H>,
    cost: Option<Arc<dyn CostInjection>>,
    redactor: Option<Arc<Redactor>>,
    framing: F,
    guard: Guard<Key>,
    items: ItemStream,
    fast_path: bool,
    str_so_far: String,
}

impl<Key, H, F> Drive<Key, H, F>
where
    Key: Send + Sync + 'static,
    H: StreamHooks<Key> + 'static,
    F: ChunkFraming,
{
    pub(super) fn start(ports: Ports<Key, H>, call: SharedCall<Key>, framing: F) -> Self {
        let SharedCall {
            source,
            key,
            request_data,
            request,
        } = call;
        let SharedSource {
            items,
            snapshot,
            buffered,
        } = source;
        let outcome = CleanupOutcome {
            request,
            request_data,
            key,
            response: snapshot,
            stream_completed: false,
            client_disconnected: false,
        };
        let items = ports
            .hooks
            .wrap_iterator(items, &outcome.key, &outcome.request_data);
        let fast_path = !ports.hooks.needs_per_chunk_hook() && ports.cost.is_none();
        Self {
            hooks: ports.hooks,
            cost: ports.cost,
            redactor: ports.redactor,
            framing,
            guard: Guard {
                outcome: Some(outcome),
                cleanup: ports.cleanup,
                reservation: ports.reservation,
                buffered,
                delivered: false,
                sealed: false,
            },
            items,
            fast_path,
            str_so_far: String::new(),
        }
    }

    pub(super) async fn pump(&mut self) -> Step<F::Out> {
        let item = match self.items.next().await {
            None => return Step::End(self.framing.frame_tail()),
            Some(Ok(item)) => item,
            Some(Err(failure)) => return Step::Failed(self.fail(failure).await),
        };
        let item = match self.prepare(item).await {
            Ok(item) => item,
            Err(failure) => return Step::Failed(self.fail(failure).await),
        };
        self.write(item)
    }

    pub(super) fn finish(&mut self, completed: bool) {
        if completed {
            self.guard.mark_completed();
        }
        self.guard.sealed = true;
    }

    async fn prepare(&mut self, item: StreamItem) -> Result<StreamItem, StreamFailure> {
        if self.fast_path {
            return Ok(item);
        }
        let hooked = match self.guard.outcome.as_ref() {
            Some(outcome) => {
                let so_far = self.str_so_far.as_str();
                self.hooks
                    .chunk_hook(item, &outcome.key, &outcome.request_data, Some(so_far))
                    .await
            }
            None => Ok(item),
        };
        let item = hooked?;
        append_str_so_far(&mut self.str_so_far, &item);
        let Some(cost) = &self.cost else {
            return Ok(item);
        };
        Ok(cost.inject(item, request_model(&self.guard)))
    }

    fn write(&mut self, item: StreamItem) -> Step<F::Out> {
        let ping = matches!(
            &item,
            StreamItem::Bytes(bytes) if bytes.as_ref() == ANTHROPIC_PING_BYTES
        );
        if !ping {
            self.guard.delivered = true;
        }
        match self.framing.frame_chunk(item) {
            Ok(out) => Step::Write(out),
            Err(abort) => {
                self.guard.sealed = true;
                Step::Failed(Err(abort))
            }
        }
    }

    async fn fail(&mut self, failure: StreamFailure) -> Result<F::Out, StreamAbort> {
        finish_failure(
            self.hooks.as_ref(),
            &mut self.framing,
            &mut self.guard,
            failure,
            self.redactor.as_deref(),
        )
        .await
    }
}

// LiteLLM's str_so_far growth: a response chunk adds its text, an event adds its content field
fn append_str_so_far(text: &mut String, item: &StreamItem) {
    let StreamItem::Chunk(chunk) = item else {
        return;
    };
    text.push_str(&chunk_response_text(chunk));
    if matches!(chunk.kind, ChunkKind::Event | ChunkKind::Plain) {
        match chunk.body.get("content") {
            None | Some(Json::Null) => {}
            Some(Json::Text(part)) => text.push_str(part),
            Some(other) => text.push_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
}

fn request_model<Key>(guard: &Guard<Key>) -> &str {
    let Some(outcome) = guard.outcome.as_ref() else {
        return "";
    };
    match outcome.request_data.get("model") {
        Some(Json::Text(text)) => text,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    }
}
