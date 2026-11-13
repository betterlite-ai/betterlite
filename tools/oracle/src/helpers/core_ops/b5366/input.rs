use serde::Deserialize;

use super::{cache, conversation, small, streams, thought, token_steps, trim_steps, validators};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}

// each group tags its own variants, so an untagged pass finds the group that knows the function name
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Call {
    Thought(thought::Call),
    Conversation(conversation::Call),
    TokenSteps(token_steps::Call),
    TrimSteps(trim_steps::Call),
    Validators(validators::Call),
    Cache(cache::Call),
    Streams(streams::Call),
    Small(small::Call),
}
