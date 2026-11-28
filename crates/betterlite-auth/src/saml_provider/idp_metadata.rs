use quick_xml::encoding::EncodingError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod extract;
pub(crate) mod parse;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdpSettings {
    pub entity_id: Option<String>,
    pub sso_url: Option<String>,
    pub slo_url: Option<String>,
    pub x509_cert: Option<String>,
    pub x509_cert_multi: Option<X509CertMulti>,
    pub name_id_format: Option<String>,
    pub want_authn_requests_signed: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct X509CertMulti {
    pub signing: Option<Vec<String>>,
    pub encryption: Option<Vec<String>>,
}

impl IdpSettings {
    // LiteLLM tests the parsed idp dict for truth; only the five idp keys count, not the sp and security overrides.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entity_id.is_none()
            && self.sso_url.is_none()
            && self.slo_url.is_none()
            && self.x509_cert.is_none()
            && self.x509_cert_multi.is_none()
    }
}

#[derive(Debug, Error)]
pub enum IdpMetadataError {
    #[error("the metadata XML could not be read")]
    Xml(#[from] quick_xml::Error),
    #[error("the metadata XML text is not valid UTF-8")]
    Text(#[from] EncodingError),
    #[error("the metadata XML declares a document type")]
    DocumentType,
    #[error("the metadata XML references the undefined entity {name}")]
    Entity { name: String },
    #[error("the metadata XML uses the undeclared namespace prefix {prefix}")]
    UnboundPrefix { prefix: String },
    #[error("the metadata XML has no root element")]
    NoRoot,
    #[error("the metadata XML ends before its root element closes")]
    Unclosed,
    #[error("the metadata XML has content after its root element")]
    Trailing,
    #[error("the metadata XML nests deeper than {limit} elements")]
    DepthExceeded { limit: usize },
    #[error("the metadata XML has more than {limit} elements")]
    TooManyElements { limit: usize },
    #[error("a metadata certificate element has no text")]
    CertificateText,
}
