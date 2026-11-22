pub mod authn_request;
pub(crate) mod driver;
pub(crate) mod runner;

mod tests;

use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::str::Utf8Error;
use std::string::FromUtf8Error;
use std::time::Duration;

use base64::DecodeError;
use quick_xml::Error as XmlError;
use serde_json::Error as JsonError;
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Error)]
pub enum SamlIdpError {
    #[error("the saml request value is too large")]
    RequestTooLarge,
    #[error("the signer input is too large")]
    InputTooLarge,
    #[error("the saml request is not valid URL-encoded text")]
    RequestText(#[source] Utf8Error),
    #[error("the saml request url is invalid: {0}")]
    RequestUrl(#[source] ParseError),
    #[error("the saml request has no SAMLRequest parameter")]
    MissingRequestParameter,
    #[error("the saml request is not valid base64: {0}")]
    RequestBase64(#[source] DecodeError),
    #[error("the saml request is not valid deflate data: {0}")]
    Inflate(#[source] io::Error),
    #[error("the saml request xml is too large")]
    XmlTooLarge,
    #[error("the saml request contains a document type declaration")]
    DocType,
    #[error("the saml request xml is not well-formed: {0}")]
    Xml(#[source] XmlError),
    #[error("the saml request root is not an AuthnRequest element")]
    NotAuthnRequest,
    #[error("the AuthnRequest has no usable root ID attribute")]
    MissingRequestId,
    #[error("the signer input is not valid JSON: {0}")]
    Json(#[source] JsonError),
    #[error("the signer could not start: {0}")]
    Spawn(#[source] io::Error),
    #[error("the signer process failed: {0}")]
    Io(#[source] io::Error),
    #[error("the signer pipe is unavailable: {0}")]
    Pipe(#[source] io::Error),
    #[error("the signer did not finish within {0:?}")]
    Timeout(Duration),
    #[error("the signer exited with {status}: {diagnostic}")]
    Exited {
        status: ExitStatus,
        diagnostic: String,
    },
    #[error("the signer output is too large")]
    OutputTooLarge,
    #[error("the signer output is not valid UTF-8: {0}")]
    Output(#[source] FromUtf8Error),
    #[error("the signer output is not valid base64: {0}")]
    ResponseBase64(#[source] DecodeError),
    #[error("the signer diagnostic is not valid UTF-8: {0}")]
    Diagnostic(#[source] FromUtf8Error),
    #[error("the fixture keypair path is not valid UTF-8")]
    FixturePath,
    #[error("the fixture keypair could not be stored at {0:?}")]
    Fixture(PathBuf),
    #[error("the signer time offsets are outside the supported range")]
    TimeRange,
}
