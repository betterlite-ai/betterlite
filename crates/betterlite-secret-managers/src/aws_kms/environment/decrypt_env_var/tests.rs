#![cfg(test)]

use std::collections::BTreeMap;

use super::{encrypted_environment, remove_secret_prefix};

#[test]
fn decrypt_env_var_rewrites_matching_environment_entries_without_mutation() {
    let source = BTreeMap::from([
        (
            "LITELLM_SECRET_AWS_KMS_TOKEN".to_owned(),
            "encoded-by-key".to_owned(),
        ),
        ("by_value".to_owned(), "aws_kms/encoded".to_owned()),
        ("ignored".to_owned(), "plain".to_owned()),
    ]);
    let selected = encrypted_environment(source.clone());
    assert_eq!(
        selected,
        vec![
            (
                "LITELLM_SECRET_AWS_KMS_TOKEN".to_owned(),
                "encoded-by-key".to_owned()
            ),
            ("by_value".to_owned(), "aws_kms/encoded".to_owned())
        ]
    );
    let rewritten: BTreeMap<String, String> = selected
        .into_iter()
        .map(|(key, value)| (remove_secret_prefix(&key), value))
        .collect();
    assert_eq!(
        rewritten,
        BTreeMap::from([
            ("TOKEN".to_owned(), "encoded-by-key".to_owned()),
            ("by_value".to_owned(), "aws_kms/encoded".to_owned())
        ])
    );
    assert_eq!(source.get("ignored"), Some(&"plain".to_owned()));
}
