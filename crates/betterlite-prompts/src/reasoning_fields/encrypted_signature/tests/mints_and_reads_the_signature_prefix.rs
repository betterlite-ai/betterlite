use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let signature = encrypted_reasoning_signature("abc123");
    assert_eq!(signature, "litellm_encrypted_reasoning:abc123");

    assert_eq!(
        encrypted_content_of_signature(&Json::Text(signature)),
        Some("abc123".to_owned())
    );
    assert_eq!(
        encrypted_content_of_signature(&Json::Text("litellm_encrypted_reasoning:".to_owned())),
        None,
        "an empty suffix after the prefix is not a carried signature"
    );
    assert_eq!(
        encrypted_content_of_signature(&Json::Text("other:abc123".to_owned())),
        None
    );
    assert_eq!(encrypted_content_of_signature(&Json::Null), None);
    assert_eq!(
        encrypted_content_of_signature(&Json::Number(serde_json::Number::from(7_i32))),
        None
    );
}
