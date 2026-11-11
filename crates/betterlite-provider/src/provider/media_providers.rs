use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaProviders;

impl Adapter for MediaProviders {
    const NAME: &'static str = "media_providers";
}
