use std::collections::BTreeMap;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures_util::Stream;

use crate::stream::chunk::{Chunk, ToolCall};

pub struct ChatGptToolCallNormalizer<S> {
    pub source: S,
    pub seen_ids: BTreeMap<String, u32>,
    pub next_index: u32,
    pub last_id: Option<String>,
}

impl<S> From<S> for ChatGptToolCallNormalizer<S> {
    fn from(source: S) -> Self {
        Self {
            source,
            seen_ids: BTreeMap::new(),
            next_index: 0,
            last_id: None,
        }
    }
}

impl<S> ChatGptToolCallNormalizer<S> {
    fn normalize(&mut self, mut chunk: Chunk) -> Option<Chunk> {
        let Some(choice) = chunk.choices.first_mut() else {
            return Some(chunk);
        };
        let Some(tool_calls) = mem::take(&mut choice.delta.tool_calls) else {
            return Some(chunk);
        };
        // LiteLLM treats an empty tool_calls list like a missing one and keeps the chunk
        if tool_calls.is_empty() {
            choice.delta.tool_calls = Some(tool_calls);
            return Some(chunk);
        }
        let normalized = self.reindex(tool_calls);
        if normalized.is_empty() {
            return None;
        }
        choice.delta.tool_calls = Some(normalized);
        Some(chunk)
    }

    fn reindex(&mut self, tool_calls: Vec<ToolCall>) -> Vec<ToolCall> {
        let mut normalized = Vec::with_capacity(tool_calls.len());
        for mut call in tool_calls {
            // the backend repeats id and name in closing chunks, and an empty id is no id at all
            let id = call.id.clone().filter(|value| !value.is_empty());
            if id
                .as_ref()
                .is_some_and(|value| self.seen_ids.contains_key(value))
            {
                continue;
            }
            if let Some(tool_id) = id {
                call.index = self.next_index;
                self.seen_ids.insert(tool_id.clone(), self.next_index);
                self.last_id = Some(tool_id);
                self.next_index = self.next_index.saturating_add(1);
                normalized.push(call);
                continue;
            }
            if let Some(index) = self
                .last_id
                .as_ref()
                .and_then(|last| self.seen_ids.get(last))
            {
                call.index = *index;
            }
            normalized.push(call);
        }
        normalized
    }
}

impl<S: Iterator<Item = Chunk>> Iterator for ChatGptToolCallNormalizer<S> {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let chunk = self.source.next()?;
            if let Some(normalized) = self.normalize(chunk) {
                return Some(normalized);
            }
        }
    }
}

impl<S: Stream<Item = Chunk> + Unpin> Stream for ChatGptToolCallNormalizer<S> {
    type Item = Chunk;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let Some(chunk) = ready!(Pin::new(&mut this.source).poll_next(cx)) else {
                return Poll::Ready(None);
            };
            if let Some(normalized) = this.normalize(chunk) {
                return Poll::Ready(Some(normalized));
            }
        }
    }
}

mod tests;
