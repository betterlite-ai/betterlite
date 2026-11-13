use crate::secret_contracts::options::SecretTags;

#[test]
fn invalid_tag_type_raises_value_error() {
    let text = serde_json::from_str::<SecretTags>("\"Environment=Prod\"")
        .expect_err("LiteLLM's ValueError is the typed decoder rejecting the tag shape");
    assert!(
        text.to_string().contains("SecretTags"),
        "the error names the tags type: {text}"
    );
    let number = serde_json::from_str::<SecretTags>(r#"{"Environment":3}"#)
        .expect_err("a dict value that is not text is rejected");
    assert!(
        number.to_string().contains("SecretTags"),
        "the error names the tags type: {number}"
    );
    let mapping = serde_json::from_str::<SecretTags>(r#"{"Environment":"Prod"}"#)
        .expect("a dict of text is the first accepted shape");
    assert!(matches!(mapping, SecretTags::Mapping(_)));
    let list = serde_json::from_str::<SecretTags>(r#"[{"Key":"Environment","Value":"Prod"}]"#)
        .expect("a list of {Key, Value} is the second accepted shape");
    assert!(matches!(list, SecretTags::List(_)));
}
