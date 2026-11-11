use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Anthropic;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicRequest;
pub struct AnthropicResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicPrepared;

impl Adapter for Anthropic {
    const NAME: &'static str = "anthropic";
}
