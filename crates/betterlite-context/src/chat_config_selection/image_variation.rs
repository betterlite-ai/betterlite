use betterlite_models::identity::provider_id::ProviderId;

use super::tags::{ConfigConstruction, ConfigSelection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageVariationConfigTag {
    OpenAi,
    Topaz,
}

impl ImageVariationConfigTag {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAIImageVariationConfig",
            Self::Topaz => "TopazImageVariationConfig",
        }
    }
}

#[must_use]
pub fn select_image_variation_config(
    _model: &str,
    provider: ProviderId,
) -> Option<ConfigSelection<ImageVariationConfigTag>> {
    if provider == ProviderId::Openai {
        return Some(ConfigSelection {
            tag: ImageVariationConfigTag::OpenAi,
            construction: ConfigConstruction::New(Vec::new()),
        });
    }
    if provider == ProviderId::Topaz {
        return Some(ConfigSelection {
            tag: ImageVariationConfigTag::Topaz,
            construction: ConfigConstruction::New(Vec::new()),
        });
    }
    None
}
