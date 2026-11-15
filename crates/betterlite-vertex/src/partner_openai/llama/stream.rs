use betterlite_openai::chat_choices::stream::CHUNK_OBJECT;
use betterlite_openai::chat_choices::wire::{StreamChoice, StreamDelta, StreamReply};

const PROVIDER_SPECIFIC_FIELDS: &str = "provider_specific_fields";

// Llama may leave the role off the first chunk, so the handler remembers the role it has sent.
#[derive(Debug, Default)]
pub struct LlamaStreamState {
    sent_role: bool,
}

impl LlamaStreamState {
    // A terminal first chunk yields the repaired first chunk, then the queued terminal chunk.
    pub fn push(&mut self, mut chunk: StreamReply) -> Vec<StreamReply> {
        if self.sent_role || chunk.choices.is_empty() {
            return vec![chunk];
        }
        self.sent_role = true;
        let terminal = chunk.choices.first_mut().and_then(repair_first_choice);
        let Some(finish_reason) = terminal else {
            return vec![chunk];
        };
        let queued = StreamReply {
            id: chunk.id.clone(),
            object: CHUNK_OBJECT,
            created: chunk.created,
            model: chunk.model.clone(),
            choices: vec![StreamChoice {
                index: 0,
                finish_reason: Some(finish_reason),
                delta: StreamDelta::default(),
                logprobs: None,
            }],
            usage: None,
            service_tier: None,
        };
        vec![chunk, queued]
    }
}

// A chunk that is first and final moves its finish reason out, keeping the role and provider fields.
fn repair_first_choice(choice: &mut StreamChoice) -> Option<String> {
    let finish_reason = choice.finish_reason.take();
    if finish_reason.is_some() {
        choice.delta.role = Some(String::from("assistant"));
        if choice.delta.content.is_none() {
            choice.delta.content = Some(String::new());
        }
        ensure_provider_fields(&mut choice.delta);
        return finish_reason;
    }
    if choice.delta.role.is_none() {
        choice.delta.role = Some(String::from("assistant"));
    }
    if choice.delta.content.as_deref().is_none_or(str::is_empty) {
        ensure_provider_fields(&mut choice.delta);
    }
    None
}

// An empty-content chunk is dropped downstream unless provider_specific_fields is a map.
fn ensure_provider_fields(delta: &mut StreamDelta) {
    let present = delta
        .passthrough
        .get(PROVIDER_SPECIFIC_FIELDS)
        .is_some_and(|value| !value.is_null());
    if !present {
        delta.passthrough.insert(
            String::from(PROVIDER_SPECIFIC_FIELDS),
            serde_json::json!({}),
        );
    }
}
