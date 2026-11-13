use std::collections::BTreeMap;

use secrecy::SecretString;

pub(super) fn read(
    environment: &BTreeMap<String, String>,
    secret_name: &str,
) -> Option<SecretString> {
    environment
        .get(secret_name)
        .map(|value| SecretString::from(value.as_str()))
}
