use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Compatible;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompatibleRequest;
pub struct CompatibleResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompatiblePrepared;

impl Adapter for Compatible {
    const NAME: &'static str = "compatible";
}
