use crate::guardrails::responses_translation::block_events::BlockStamp;

// LiteLLM draws a block message's ids from uuid4 and its created time from the clock; the caller owns both.
pub trait StampSource {
    fn stamp(&self) -> BlockStamp;
}
