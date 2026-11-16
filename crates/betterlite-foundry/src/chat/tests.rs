#![cfg(test)]

mod capabilities;
mod cases;
mod credentials;
mod environment;
mod fields;
mod messages;
mod provider;
mod request;
mod response;
mod support;
mod url;

#[test]
fn foundry_chat_credential_identity_isolation() {
    credentials::credential_identity_isolation();
}

#[test]
fn foundry_chat_secret_safe_failure() {
    credentials::secret_safe_failure();
}
