use crate::image_generation::error::FoundryImageError;
use crate::image_generation::flux::{flux2_image_generation_url, is_flux2_model};

const FLEX: &str = "FLUX.2-flex";
const BASE: &str = "https://example.test";

pub(super) fn flux2_model_route() {
    flux2_detection();
    appended_provider_path();
    provider_base_kept();
    missing_api_base();
}

fn flux2_detection() {
    assert!(is_flux2_model("flux.2-pro"));
    assert!(is_flux2_model("FLUX-2-flex"));
    assert!(is_flux2_model("azure_ai/FLUX_2_max"));
    assert!(is_flux2_model("black-forest-labs/FLUX2-dev"));
    assert!(is_flux2_model("flux2"));
    assert!(!is_flux2_model("flux-1-schnell"));
    assert!(!is_flux2_model("fluxpro"));
    assert!(!is_flux2_model("FLUX.1-pro"));
    assert!(!is_flux2_model(""));
}

fn appended_provider_path() {
    assert_eq!(
        flux2_image_generation_url(Some(BASE), FLEX, None).expect("a url"),
        "https://example.test/providers/blackforestlabs/v1/flux-2-flex?api-version=preview"
    );
    assert_eq!(
        flux2_image_generation_url(Some(BASE), FLEX, Some("")).expect("a url"),
        "https://example.test/providers/blackforestlabs/v1/flux-2-flex?api-version=preview"
    );
    assert_eq!(
        flux2_image_generation_url(Some(BASE), "flux.2-pro", Some("2025-04-01")).expect("a url"),
        "https://example.test/providers/blackforestlabs/v1/flux-2-pro?api-version=2025-04-01"
    );
    assert_eq!(
        flux2_image_generation_url(Some("https://example.test///"), FLEX, None).expect("a url"),
        "https://example.test/providers/blackforestlabs/v1/flux-2-flex?api-version=preview"
    );
}

fn provider_base_kept() {
    assert_eq!(
        flux2_image_generation_url(
            Some("https://example.test/providers/blackforestlabs/v1/flux-2-pro/"),
            "flux.2-pro",
            None,
        )
        .expect("a url"),
        "https://example.test/providers/blackforestlabs/v1/flux-2-pro?api-version=preview"
    );
    assert_eq!(
        flux2_image_generation_url(
            Some("https://example.test/edge/providers/blackforestlabs/v1/flux-2-pro?api-version=1"),
            "flux.2-pro",
            Some("2025-04-01"),
        )
        .expect("a url"),
        "https://example.test/edge/providers/blackforestlabs/v1/flux-2-pro?api-version=1"
    );
    assert_eq!(
        flux2_image_generation_url(
            Some("https://example.test/providers/x/p?x=1&y=2"),
            "flux.2-pro",
            None,
        )
        .expect("a url"),
        "https://example.test/providers/x/p?x=1&y=2"
    );
}

fn missing_api_base() {
    let error = flux2_image_generation_url(None, FLEX, None).expect_err("api_base is required");
    assert_eq!(error, FoundryImageError::ApiBaseRequired);
    assert_eq!(error.status_code(), None);
    assert_eq!(error.llm_provider(), None);
    assert_eq!(
        error.to_string(),
        "api_base is required for Azure AI FLUX 2 image generation"
    );
}
