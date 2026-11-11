use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gemini;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GeminiRequest;
pub struct GeminiResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GeminiPrepared;

impl Adapter for Gemini {
    const NAME: &'static str = "gemini";
}
