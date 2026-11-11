use std::io;

use jsonwebtoken::errors::Error as JwtError;
use p12_keystore::error::Error as Pkcs12Error;
use pem::PemError;
use pkcs8::Error as Pkcs8Error;
use thiserror::Error;

use crate::azure_client_secret::ClientSecretError;

pub mod assertion;
pub mod credential;
pub mod exchange;
pub mod material;

mod tests;

pub const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

// the library error that made a PKCS#12 keystore unreadable
#[derive(Debug, Error)]
pub enum DecodeFailure {
    #[error(transparent)]
    Pkcs12(#[from] Pkcs12Error),
    #[error(transparent)]
    Pkcs8(#[from] Pkcs8Error),
}

// the messages are azure-identity's and cryptography's, which callers of LiteLLM read today
#[derive(Debug, Error)]
pub enum CertificateError {
    #[error(transparent)]
    Client(#[from] ClientSecretError),
    #[error("Please specify either \"certificate_path\" or \"certificate_data\", not both")]
    BothSources,
    #[error(
        "CertificateCredential requires a value for either \"certificate_path\" or \"certificate_data\""
    )]
    NoSource,
    #[error("unable to read the certificate file {path}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("Failed to deserialize certificate in PEM or PKCS12 format")]
    Deserialize {
        #[source]
        source: DecodeFailure,
    },
    #[error("Failed to deserialize certificate in PEM or PKCS12 format")]
    NoSigningCertificate,
    #[error(
        "Unable to load PEM file. See https://cryptography.io/en/latest/faq/#why-can-t-i-import-my-pem-file for more details. MalformedFraming"
    )]
    PemFraming {
        #[source]
        source: PemError,
    },
    #[error(
        "Unable to load PEM file. See https://cryptography.io/en/latest/faq/#why-can-t-i-import-my-pem-file for more details. MalformedFraming"
    )]
    NoPemBlock,
    #[error(
        "Could not deserialize key data. The data may be in an incorrect format, it may be encrypted with an unsupported algorithm, or it may be an unsupported key type (e.g. EC curves with explicit parameters)."
    )]
    KeyData {
        #[source]
        source: Pkcs8Error,
    },
    #[error(
        "Could not deserialize key data. The data may be in an incorrect format, it may be encrypted with an unsupported algorithm, or it may be an unsupported key type (e.g. EC curves with explicit parameters)."
    )]
    UnsupportedKeyBlock,
    #[error(
        "Valid PEM but no BEGIN/END delimiters for a private key found. Are you sure this is a private key?"
    )]
    NoPrivateKeyBlock,
    #[error(
        "Valid PEM but no BEGIN CERTIFICATE/END CERTIFICATE delimiters. Are you sure this is a certificate?"
    )]
    NoCertificateBlock,
    #[error("Authentication failed: {message}")]
    Refused { message: String },
    #[error("The certificate must include its private key")]
    MissingPrivateKey,
    #[error("The certificate must have an RSA private key because RS256 is used for signing")]
    NotRsa,
    #[error("Password was not given but private key is encrypted")]
    PasswordRequired,
    #[error("Password was given but private key is not encrypted.")]
    PasswordUnexpected,
    #[error("Incorrect password, could not decrypt key")]
    BadPassword {
        #[source]
        source: Pkcs8Error,
    },
    #[error("a PEM private key encrypted with a Proc-Type header is not supported")]
    LegacyEncryptedPem,
    #[error("the RSA private key cannot sign client assertions")]
    Sign {
        #[source]
        source: JwtError,
    },
}
