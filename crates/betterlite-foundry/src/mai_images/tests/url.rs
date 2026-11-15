use crate::mai_images::error::MaiImageError;
use crate::mai_images::url::{ImageOperation, is_mai_model, mai_image_url};

const BASE: &str = "https://hub.services.ai.azure.com";

pub(super) fn mai_generation_edit_url() {
    let missing = mai_image_url(ImageOperation::Generation, None, None).unwrap_err();
    assert!(matches!(
        missing,
        MaiImageError::ApiBaseRequired {
            operation: "generation"
        }
    ));
    assert_eq!(
        missing.to_string(),
        "api_base is required for Azure AI MAI image generation"
    );
    assert_eq!(missing.status_code(), None);
    let missing_edit = mai_image_url(ImageOperation::Edit, None, Some("v1")).unwrap_err();
    assert_eq!(
        missing_edit.to_string(),
        "api_base is required for Azure AI MAI image editing"
    );
    let generation = |base, version| mai_image_url(ImageOperation::Generation, base, version);
    let edit = |base, version| mai_image_url(ImageOperation::Edit, base, version);
    assert_eq!(
        generation(Some(BASE), None).unwrap(),
        format!("{BASE}/mai/v1/images/generations?api-version=preview")
    );
    assert_eq!(
        edit(Some(BASE), Some("")).unwrap(),
        format!("{BASE}/mai/v1/images/edits?api-version=preview")
    );
    assert_eq!(
        edit(Some("https://hub.example//"), Some("2025-11-15")).unwrap(),
        "https://hub.example/mai/v1/images/edits?api-version=2025-11-15"
    );
    assert_eq!(
        generation(Some("https://hub.example/mai/v1/images/edits?x=1"), None).unwrap(),
        "https://hub.example/mai/v1/images/generations?x=1"
    );
    assert_eq!(
        edit(Some("https://hub.example/mai/v1/images/generations/"), None).unwrap(),
        "https://hub.example/mai/v1/images/edits?api-version=preview"
    );
    assert_eq!(
        edit(Some("https://hub.example/mai/v2"), Some("a")).unwrap(),
        "https://hub.example/mai/v2/images/edits?api-version=a"
    );
    assert_eq!(
        generation(Some("https://hub.example/a/images/b/mai/images/c"), None).unwrap(),
        "https://hub.example/a/images/generations?api-version=preview"
    );
    assert_eq!(
        generation(Some("https://hub.example?"), Some("v")).unwrap(),
        "https://hub.example/mai/v1/images/generations?"
    );
    assert_eq!(
        generation(Some("https://hub.example/?a=b?c"), Some("v")).unwrap(),
        "https://hub.example/mai/v1/images/generations?a=b?c"
    );
    family_detector();
}

fn family_detector() {
    assert!(is_mai_model("MAI-Image-2.5"));
    assert!(is_mai_model("azure_ai/mai_image-1"));
    assert!(is_mai_model("MAIIMAGE"));
    assert!(!is_mai_model("gpt-image-1"));
    assert!(!is_mai_model("mai-voice-1"));
}
