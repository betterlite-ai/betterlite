#![expect(
    clippy::disallowed_types,
    reason = "a buffered Anthropic SSE event is provider JSON that betterlite does not model"
)]

pub mod ended;
pub mod events;
pub mod json_dumps;
pub mod line;
pub mod rewriter;

mod tests;
