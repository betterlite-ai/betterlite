use secrecy::{ExposeSecret as _, SecretString};

#[must_use]
pub fn sanitize(message: &str, secrets: &[Option<&SecretString>]) -> String {
    let mut sanitized = message.to_owned();
    for secret in secrets.iter().flatten() {
        let value = secret.expose_secret();
        if !value.is_empty() {
            sanitized = sanitized.replace(value, "***");
        }
    }
    sanitized
}
