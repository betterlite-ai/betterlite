use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeechProviders;

impl Adapter for SpeechProviders {
    const NAME: &'static str = "speech_providers";
}
