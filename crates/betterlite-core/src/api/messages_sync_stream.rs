// LiteLLM's synchronous AnthropicStreamWrapper.__next__: one transition per call, with the provider pull outside it.
pub mod driver;
pub mod engine;
mod frames;
mod translate;

mod tests;
