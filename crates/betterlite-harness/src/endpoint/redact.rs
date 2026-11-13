#[must_use]
pub fn sanitize(message: &str, secrets: &[Option<&str>]) -> String {
    let mut sanitized = message.to_owned();
    for secret in secrets.iter().flatten() {
        if !secret.is_empty() {
            sanitized = sanitized.replace(secret, "***");
        }
    }
    sanitized
}
