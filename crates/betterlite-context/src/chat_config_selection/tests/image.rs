use betterlite_models::identity::provider_id::ProviderId;

use super::super::image_variation::{ImageVariationConfigTag, select_image_variation_config};
use super::super::tags::{ConfigConstruction, ConfigSelection};

pub(super) fn image_variation_config_selection() {
    image_variation_chosen();
    image_variation_absent();
    assert_eq!(
        ImageVariationConfigTag::OpenAi.class_name(),
        "OpenAIImageVariationConfig"
    );
    assert_eq!(
        ImageVariationConfigTag::Topaz.class_name(),
        "TopazImageVariationConfig"
    );
}

fn image_variation_chosen() {
    for model in ["dall-e-2", "gpt-image-1", ""] {
        assert_eq!(
            select_image_variation_config(model, ProviderId::Openai),
            Some(ConfigSelection {
                tag: ImageVariationConfigTag::OpenAi,
                construction: ConfigConstruction::New(Vec::new()),
            }),
            "{model}"
        );
    }
    for model in ["topaz-upscale", "anything", ""] {
        assert_eq!(
            select_image_variation_config(model, ProviderId::Topaz),
            Some(ConfigSelection {
                tag: ImageVariationConfigTag::Topaz,
                construction: ConfigConstruction::New(Vec::new()),
            }),
            "{model}"
        );
    }
}

fn image_variation_absent() {
    for provider in [
        ProviderId::Groq,
        ProviderId::Azure,
        ProviderId::AzureAi,
        ProviderId::Publicai,
    ] {
        assert_eq!(
            select_image_variation_config("dall-e-2", provider),
            None,
            "{provider:?}"
        );
    }
}
