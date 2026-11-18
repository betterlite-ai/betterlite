pub mod base;
pub mod config;

use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Anthropic;

// B010's stubs stay in the shape their variants were registered with until the composition fills them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicRequest;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicPrepared;

impl Adapter for Anthropic {
    const NAME: &'static str = "anthropic";
}
