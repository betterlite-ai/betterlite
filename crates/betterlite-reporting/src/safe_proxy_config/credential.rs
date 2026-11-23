pub(crate) const CREDENTIAL_KEY_EXCEPTIONS: [&str; 1] = ["key_management_system"];

pub(crate) const CREDENTIAL_KEY_PARTS: [&str; 19] = [
    "key",
    "keys",
    "secret",
    "secrets",
    "token",
    "password",
    "passwd",
    "credential",
    "credentials",
    "url",
    "uri",
    "dsn",
    "host",
    "hosts",
    "base",
    "endpoint",
    "cert",
    "pem",
    "salt",
];

pub(crate) fn is_credential_key(key: &str) -> bool {
    if CREDENTIAL_KEY_EXCEPTIONS.contains(&key) {
        return false;
    }
    key.to_lowercase()
        .split('_')
        .any(|part| CREDENTIAL_KEY_PARTS.contains(&part))
}
