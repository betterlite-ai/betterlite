use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XaiMistral;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XaiMistralRequest;
pub struct XaiMistralResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XaiMistralPrepared;

impl Adapter for XaiMistral {
    const NAME: &'static str = "xai_mistral";
}
