use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bedrock;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockRequest;
pub struct BedrockResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockPrepared;

impl Adapter for Bedrock {
    const NAME: &'static str = "bedrock";
}
