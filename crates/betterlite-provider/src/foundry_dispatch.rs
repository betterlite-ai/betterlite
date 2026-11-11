use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureAi;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureAiRequest;
pub struct AzureAiResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureAiPrepared;

impl Adapter for AzureAi {
    const NAME: &'static str = "azure_ai";
}
