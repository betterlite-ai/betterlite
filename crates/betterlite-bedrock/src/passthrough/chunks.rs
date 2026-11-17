use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk, StreamDelta};
#[must_use]
pub fn plain_text_delta(chunk: &StreamChunk) -> Option<&str> {
    let [choice] = chunk.choices.as_slice() else {
        return None;
    };
    let blocked = chunk.usage.is_some()
        || chunk
            .provider_specific_fields
            .as_ref()
            .is_some_and(|fields| !fields.is_empty())
        || choice
            .finish_reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty())
        || choice.logprobs.is_some()
        || delta_carries_more_than_text(&choice.delta);
    if blocked {
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
    if !parts.is_empty()
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
        if let Some(text) = plain_text_delta(&chunk)
            && !self.open_text_parts.is_empty()
        {
            self.open_text_parts.push(text.to_owned());
            return;
        }
        self.seal_text_run();
        if let Some(text) = plain_text_delta(&chunk) {
            self.open_text_parts.push(text.to_owned());
        }
        self.chunks.push(chunk);
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
