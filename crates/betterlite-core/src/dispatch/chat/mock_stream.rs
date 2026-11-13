use betterlite_config::sections::mock_response::UsageDefaults;
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};

use crate::mock::stream::{StreamSpec, chunks, usage_chunk};

mod tests;

// LiteLLM's stream wrapper keeps the role on the first delta and on choices after the first
pub fn mock_frames(
    spec: &StreamSpec<'_>,
    request: &ClientChatRequest,
    prompt_tokens: Option<u64>,
    usage: UsageDefaults,
) -> Result<Vec<Vec<u8>>, serde_json::Error> {
    let mut frames = Vec::new();
    for (position, mut chunk) in chunks(spec).enumerate() {
        if let Some(first) = chunk.choices.first_mut().filter(|_| position > 0) {
            first.delta.role = None;
        }
        // the wrapper drops a chunk with no choice, which is every content chunk when n is zero
        if chunk.choices.is_empty() {
            continue;
        }
        frames.push(sse_frame(&chunk)?);
    }
    frames.push(sse_frame(&Chunk {
        id: spec.id.to_owned(),
        object: String::from("chat.completion.chunk"),
        created: spec.created,
        model: spec.model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta: Delta::default(),
            finish_reason: Some(String::from("stop")),
            logprobs: None,
        }],
        usage: None,
        system_fingerprint: None,
        service_tier: None,
    })?);
    let with_usage = request
        .stream_options
        .as_ref()
        .is_some_and(|options| options.include_usage);
    if with_usage || prompt_tokens.is_some() {
        let count = prompt_tokens.unwrap_or(usage.prompt_tokens);
        frames.push(sse_frame(&usage_chunk(spec, count, usage))?);
    }
    frames.push(b"data: [DONE]\n\n".to_vec());
    Ok(frames)
}

pub fn sse_frame(chunk: &Chunk) -> Result<Vec<u8>, serde_json::Error> {
    let text = serde_json::to_string(chunk)?;
    Ok(format!("data: {text}\n\n").into_bytes())
}
