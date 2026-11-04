#![cfg(test)]

use super::remove_secret_prefix;

#[test]
fn decrypt_env_var_rewrites_matching_environment_entries_without_mutation() {
    assert_eq!(
        remove_secret_prefix("LITELLM_SECRET_AWS_KMS_TOKEN"),
        "TOKEN"
    );
    assert_eq!(remove_secret_prefix("ordinary"), "ordinary");
}
