use crate::provider::Adapter;

pub mod call;
pub mod chat;
pub mod context;
pub mod embedding;
pub mod error;
pub mod handler;
pub mod lemonade;
pub mod logging;
pub mod ollama_models;
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelfHosted;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelfHostedRequest;
pub struct SelfHostedResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelfHostedPrepared;

impl Adapter for SelfHosted {
    const NAME: &'static str = "self_hosted";
}
