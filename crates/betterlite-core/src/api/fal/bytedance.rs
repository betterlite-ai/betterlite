mod tests;

use crate::api::fal::flux::FluxProfile;

// both models map parameters like Flux Schnell (image_size, jpeg output, num_images) and reply like any Flux profile
pub const MAPPING_PROFILE: FluxProfile = FluxProfile::Schnell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BytedanceModel {
    SeedreamV3,
    DreaminaV31,
}

impl BytedanceModel {
    #[must_use]
    pub const fn endpoint(self) -> &'static str {
        match self {
            Self::SeedreamV3 => "fal-ai/bytedance/seedream/v3/text-to-image",
            Self::DreaminaV31 => "fal-ai/bytedance/dreamina/v3.1/text-to-image",
        }
    }
}
