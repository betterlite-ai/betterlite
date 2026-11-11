use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Azure;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureRequest;
pub struct AzureResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzurePrepared;

impl Adapter for Azure {
    const NAME: &'static str = "azure";
}
