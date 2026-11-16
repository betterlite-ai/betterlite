use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk, StreamDelta};
#[must_use]
pub fn plain_text_delta(chunk: &StreamChunk) -> Option<&str> {
    if chunk.usage.is_some() {
        return None;
    }
    if chunk
        .provider_specific_fields
        .as_ref()
        .is_some_and(|fields| !fields.is_empty())
    {
        return None;
    }
    let [choice] = chunk.choices.as_slice() else {
        return None;
    };
    if choice
        .finish_reason
        .as_deref()
        .is_some_and(|reason| !reason.is_empty())
    {
        return None;
    }
    if choice.logprobs.is_some() {
        return None;
    }
    if delta_carries_more_than_text(&choice.delta) {
        return None;
    }
    choice.delta.content.as_deref()
}
fn delta_carries_more_than_text(delta: &StreamDelta) -> bool {
    delta.function_call.is_some()
        || delta.tool_calls.is_some()
        || delta.audio.is_some()
        || delta.images.is_some()
        || delta.annotations.is_some()
        || delta.reasoning_content.is_some()
        || delta.thinking_blocks.is_some()
        || delta.provider_specific_fields.is_some()
        || delta
            .extra
            .values()
            .any(|value| !matches!(value, JsonValue::Null))
}
fn seal_run(chunks: &mut [StreamChunk], parts: &[String]) {
    if parts.len() > 1
        && let Some(choice) = chunks
            .last_mut()
            .and_then(|chunk| chunk.choices.first_mut())
    {
        choice.delta.content = Some(parts.concat());
    }
}
#[derive(Debug, Default)]
pub struct CoalescedChunks {
    chunks: Vec<StreamChunk>,
    open_text_parts: Vec<String>,
}
impl CoalescedChunks {
    pub fn add(&mut self, chunk: StreamChunk) {
        let text = plain_text_delta(&chunk).map(str::to_owned);
        if let Some(text) = &text
            && !self.open_text_parts.is_empty()
        {
            self.open_text_parts.push(text.clone());
            return;
        }
        self.seal_text_run();
        self.chunks.push(chunk);
        if let Some(text) = text {
            self.open_text_parts.push(text);
        }
    }
    fn seal_text_run(&mut self) {
        seal_run(&mut self.chunks, &self.open_text_parts);
        self.open_text_parts.clear();
    }
    pub fn chunks(&mut self) -> &[StreamChunk] {
        self.seal_text_run();
        &self.chunks
    }
    #[must_use]
    pub fn snapshot(&self) -> Vec<StreamChunk> {
        let mut chunks = self.chunks.clone();
        seal_run(&mut chunks, &self.open_text_parts);
        chunks
    }
}
