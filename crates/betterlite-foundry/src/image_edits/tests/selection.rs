use crate::image_edits::error::ImageEditError;
use crate::image_edits::selection::ImageEditVariant;

use super::support::{FLUX1, FLUX2, MAI};

pub(super) fn image_edit_model_selection() {
    model_names();
    other_models();
    class_names_and_transports();
}

fn model_names() {
    let cases = [
        (MAI, ImageEditVariant::Mai),
        ("mai-image-2.5", ImageEditVariant::Mai),
        ("MAI_Image_2", ImageEditVariant::Mai),
        ("azure/mai-image-2.5", ImageEditVariant::Mai),
        (FLUX2, ImageEditVariant::Flux2),
        ("flux.2-flex", ImageEditVariant::Flux2),
        ("FLUX-2-kontext", ImageEditVariant::Flux2),
        ("flux2", ImageEditVariant::Flux2),
        (FLUX1, ImageEditVariant::Flux1),
        ("flux", ImageEditVariant::Flux1),
        ("FLUX-1-kontext-max", ImageEditVariant::Flux1),
        ("", ImageEditVariant::Flux1),
        ("-_", ImageEditVariant::Flux1),
    ];
    for (model, expected) in cases {
        assert_eq!(
            ImageEditVariant::select(model).unwrap(),
            expected,
            "{model}"
        );
    }
    // a MAI name still wins over a FLUX 2 name when it carries both
    assert_eq!(
        ImageEditVariant::select("MAI-image-flux-2").unwrap(),
        ImageEditVariant::Mai
    );
}

fn other_models() {
    let error = ImageEditVariant::select("gpt-image-1").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model gpt-image-1 is not supported for Azure AI image editing."
    );
    assert!(matches!(error, ImageEditError::UnsupportedModel { .. }));
    assert_eq!(error.status_code(), None);

    assert!(ImageEditVariant::select("dall-e-3").is_err());
    assert!(ImageEditVariant::select("stable-diffusion").is_err());
}

pub(super) fn class_names_and_transports() {
    for (variant, class_name, multipart) in [
        (
            ImageEditVariant::Flux1,
            "AzureFoundryFluxImageEditConfig",
            true,
        ),
        (
            ImageEditVariant::Flux2,
            "AzureFoundryFlux2ImageEditConfig",
            false,
        ),
        (
            ImageEditVariant::Mai,
            "AzureFoundryMAIImageEditConfig",
            true,
        ),
    ] {
        assert_eq!(variant.class_name(), class_name);
        assert_eq!(variant.uses_multipart(), multipart, "{class_name}");
    }
}
