use crate::image_generation::flux::flux2_provider_model_path;

pub(super) fn flux_model_path() {
    assert_eq!(flux2_provider_model_path("FLUX.2-flex"), "flux-2-flex");
    assert_eq!(flux2_provider_model_path("flux_2_flex"), "flux-2-flex");
    assert_eq!(
        flux2_provider_model_path("azure_ai/FLUX.2-FLEX-preview"),
        "flux-2-flex"
    );
    assert_eq!(
        flux2_provider_model_path("black-forest-labs/flux-2-flex"),
        "flux-2-flex"
    );
    assert_eq!(flux2_provider_model_path("flux-2-flexible"), "flux-2-flex");
    assert_eq!(flux2_provider_model_path("flux2-flex"), "flux-2-pro");
    assert_eq!(flux2_provider_model_path("flux.2-pro"), "flux-2-pro");
    assert_eq!(flux2_provider_model_path("flux-2"), "flux-2-pro");
    assert_eq!(flux2_provider_model_path("flux.2-dev"), "flux-2-pro");
    assert_eq!(flux2_provider_model_path("gpt-image-1"), "flux-2-pro");
    assert_eq!(flux2_provider_model_path(""), "flux-2-pro");
}
