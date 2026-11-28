mod authn_request;
pub mod cache;
pub mod error;
pub mod fetch;
pub mod idp_metadata;
pub mod provider;
pub mod settings;
pub mod sp_metadata;
mod tests;
pub mod verify;

pub(crate) const SAML_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";
pub(crate) const MD_NAMESPACE: &str = "urn:oasis:names:tc:SAML:2.0:metadata";
pub(crate) const DS_NAMESPACE: &str = "http://www.w3.org/2000/09/xmldsig#";
pub(crate) const SAML_PROTOCOL_NAMESPACE: &str = "urn:oasis:names:tc:SAML:2.0:protocol";

fn timestamp(seconds: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0)
        .map(|moment| moment.format(SAML_TIME_FORMAT).to_string())
}
