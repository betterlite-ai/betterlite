use crate::provider::Adapter;

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
