use super::support::{fake, fake_in};

const REDUCED: &str = r#"{"azure/gpt-5.1":{"supports_reasoning":true,"supports_none_reasoning_effort":true,"default_reasoning_effort":"none"}}"#;

pub(super) fn o3_native_stream() {
    assert!(!fake(Some("o3-mini"), Some(true), Some("azure")));
    assert!(!fake(Some("azure-o3-deployment"), Some(true), None));
    assert!(!fake(Some("o1-preview"), Some(true), Some("azure")));
    assert!(fake(Some("o1-mini"), Some(true), Some("azure")));
    assert!(!fake(Some("o1-mini"), Some(false), Some("azure")));
    assert!(!fake(Some("o1-mini"), None, Some("azure")));
    assert!(fake(None, Some(true), Some("azure")));
    assert!(fake(Some("o1-mini"), Some(true), None));
    assert!(fake(Some(""), Some(true), Some("azure")));
}

pub(super) fn unknown_o1_fake_stream() {
    assert!(fake_in(
        REDUCED,
        Some("my-o1-deployment"),
        Some(true),
        Some("azure")
    ));
    assert!(fake_in(REDUCED, Some("o1-preview"), Some(true), None));
    assert!(fake_in(REDUCED, Some("o1"), Some(true), Some("azure")));
    assert!(!fake_in(
        REDUCED,
        Some("o3-preview"),
        Some(true),
        Some("azure")
    ));
}
