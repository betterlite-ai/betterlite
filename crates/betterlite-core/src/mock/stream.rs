use betterlite_config::sections::mock_response::UsageDefaults;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};

pub struct StreamSpec<'stream> {
    pub content: &'stream str,
    pub choice_count: Option<u32>,
    pub model: &'stream str,
    pub id: &'stream str,
    pub created: u64,
}

const CHUNK_CHARACTERS: usize = 3;

// mock_completion_streaming_obj slices the content into three-character deltas, as Python slicing does
pub fn chunks<'stream>(
    spec: &'stream StreamSpec<'stream>,
) -> impl Iterator<Item = Chunk> + 'stream {
    let characters: Vec<char> = spec.content.chars().collect();
    let groups = characters.len().div_ceil(CHUNK_CHARACTERS);
    (0..groups).map(move |group| {
        let content: String = characters
            .iter()
            .skip(group.saturating_mul(CHUNK_CHARACTERS))
            .take(CHUNK_CHARACTERS)
            .collect();
        Chunk {
            id: spec.id.to_owned(),
            object: String::from("chat.completion.chunk"),
            created: spec.created,
            model: spec.model.to_owned(),
            choices: delta_choices(spec.choice_count, &content),
            usage: None,
            system_fingerprint: None,
            service_tier: None,
        }
    })
}

fn delta_choices(choice_count: Option<u32>, content: &str) -> Vec<Choice> {
    (0..choice_count.unwrap_or(1))
        .map(|index| Choice {
            index,
            delta: Delta {
                role: Some(String::from("assistant")),
                content: Some(content.to_owned()),
                refusal: None,
                tool_calls: None,
                function_call: None,
            },
            finish_reason: None,
            logprobs: None,
        })
        .collect()
}

// mock_stream_usage_chunk reports the prompt count the caller injected and the completion default
#[must_use]
pub fn usage_chunk(spec: &StreamSpec<'_>, prompt_tokens: u64, usage: UsageDefaults) -> Chunk {
    let completion_tokens = usage.completion_tokens;
    Chunk {
        id: spec.id.to_owned(),
        object: String::from("chat.completion.chunk"),
        created: spec.created,
        model: spec.model.to_owned(),
        choices: Vec::new(),
        usage: Some(ChatUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }),
        system_fingerprint: None,
        service_tier: None,
    }
}

mod tests;
