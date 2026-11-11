use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Legacy;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyRequest;
pub struct LegacyResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyPrepared;

impl Adapter for Legacy {
    const NAME: &'static str = "legacy";
}
