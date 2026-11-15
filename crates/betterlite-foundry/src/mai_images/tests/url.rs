use crate::mai_images::error::MaiImageError;
use crate::mai_images::url::{ImageOperation, is_mai_model, mai_image_url};

const BASE: &str = "https://hub.services.ai.azure.com";
const HUB: &str = "https://hub.example";

fn generation(base: &str, version: Option<&str>) -> String {
    mai_image_url(ImageOperation::Generation, Some(base), version).unwrap()
}

fn edit(base: &str, version: Option<&str>) -> String {
    mai_image_url(ImageOperation::Edit, Some(base), version).unwrap()
}

pub(super) fn mai_generation_edit_url() {
    missing_base();
    appended_path();
    existing_mai_path();
    original_query();
    family_detector();
}

fn missing_base() {
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
    assert_eq!(missing.llm_provider(), None);
    let missing_edit = mai_image_url(ImageOperation::Edit, None, Some("v1")).unwrap_err();
    assert_eq!(
        missing_edit.to_string(),
        "api_base is required for Azure AI MAI image editing"
    );
}

fn appended_path() {
    let generated = format!("{BASE}/mai/v1/images/generations?api-version=preview");
    assert_eq!(generation(BASE, None), generated);
    assert_eq!(generation(BASE, Some("")), generated);
    assert_eq!(
        edit(BASE, Some("")),
        format!("{BASE}/mai/v1/images/edits?api-version=preview")
    );
    assert_eq!(
        edit(&format!("{HUB}//"), Some("2025-11-15")),
        format!("{HUB}/mai/v1/images/edits?api-version=2025-11-15")
    );
}

fn existing_mai_path() {
    let preview = "?api-version=preview";
    assert_eq!(
        edit(&format!("{HUB}/mai/v1/images/generations/"), None),
        format!("{HUB}/mai/v1/images/edits{preview}")
    );
    assert_eq!(
        edit(&format!("{HUB}/mai/v2"), Some("a")),
        format!("{HUB}/mai/v2/images/edits?api-version=a")
    );
    assert_eq!(
        generation(&format!("{HUB}/a/images/b/mai/images/c"), None),
        format!("{HUB}/a/images/generations{preview}")
    );
}

fn original_query() {
    assert_eq!(
        generation(&format!("{HUB}/mai/v1/images/edits?x=1"), None),
        format!("{HUB}/mai/v1/images/generations?x=1")
    );
    assert_eq!(
        generation(&format!("{HUB}?"), Some("v")),
        format!("{HUB}/mai/v1/images/generations?")
    );
    assert_eq!(
        generation(&format!("{HUB}/?a=b?c"), Some("v")),
        format!("{HUB}/mai/v1/images/generations?a=b?c")
    );
}

fn family_detector() {
    assert!(is_mai_model("MAI-Image-2.5"));
    assert!(is_mai_model("azure_ai/mai_image-1"));
    assert!(is_mai_model("MAIIMAGE"));
    assert!(!is_mai_model("gpt-image-1"));
    assert!(!is_mai_model("mai-voice-1"));
}
