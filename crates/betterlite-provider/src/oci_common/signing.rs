use std::fs;
use std::io::ErrorKind;

use aws_lc_rs::digest::{self, SHA256};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeyPair;
use aws_lc_rs::signature::RSA_PKCS1_SHA256;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, DATE, HOST, HeaderName};
use http::{HeaderMap, HeaderValue};
use serde::Serialize;
use url::Url;

use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::{Clock, OciError};

const MISSING_CREDENTIALS: &str = "Missing required OCI credentials: oci_user, oci_fingerprint, oci_tenancy, and at least one of oci_key or oci_key_file. These can also be supplied via environment variables: OCI_USER, OCI_FINGERPRINT, OCI_TENANCY, OCI_KEY (or OCI_KEY_FILE). Alternatively, provide an oci_signer object from the OCI SDK.";
const NOT_RSA: &str =
    "The provided private key is not an RSA key, which is required for OCI signing.";
const SIGNED_HEADER_LIST: &str =
    "date (request-target) host content-length content-type x-content-sha256";
const RFC_7231_DATE: &str = "%a, %d %b %Y %H:%M:%S GMT";
const CONTENT_SHA256: HeaderName = HeaderName::from_static("x-content-sha256");

pub fn sign_oci_request<T>(
    headers: &mut HeaderMap,
    credentials: &OciCredentials,
    request: &T,
    api_base: &str,
    clock: &dyn Clock,
) -> Result<Vec<u8>, OciError>
where
    T: Serialize,
{
    let key = signing_key(credentials)?;
    let body = serde_json::to_vec(request).map_err(|source| OciError {
        status: 500,
        message: format!("the OCI request body could not be encoded: {source}"),
    })?;
    let parsed = Url::parse(api_base).map_err(|source| OciError {
        status: 500,
        message: format!("the OCI api base is not a URL: {source}"),
    })?;
    let host = parsed.authority();
    let path = if parsed.path().is_empty() {
        "/"
    } else {
        parsed.path()
    };
    let date = clock.now().format(RFC_7231_DATE).to_string();
    let content_type = content_type(headers)?;
    let content_length = body.len().to_string();
    let content_sha256 = STANDARD.encode(digest::digest(&SHA256, &body).as_ref());
    let subject = SignatureSubject {
        date: &date,
        host,
        path,
        content_type: &content_type,
        content_length: &content_length,
        content_sha256: &content_sha256,
    };
    let signature = sign_string(&key.key_pair, &signing_string(&subject))?;
    let signed = SignedHeaders {
        authorization: authorization(&key.key_id, &signature),
        date,
        host: host.to_owned(),
        content_type,
        content_length,
        content_sha256,
    };
    insert_signed_headers(headers, &signed)?;
    Ok(body)
}

struct SigningKey {
    key_id: String,
    key_pair: KeyPair,
}

fn signing_key(credentials: &OciCredentials) -> Result<SigningKey, OciError> {
    let user_value = credentials
        .oci_user
        .as_deref()
        .filter(|value| !value.is_empty());
    let fingerprint_value = credentials
        .oci_fingerprint
        .as_deref()
        .filter(|value| !value.is_empty());
    let tenancy_value = credentials
        .oci_tenancy
        .as_deref()
        .filter(|value| !value.is_empty());
    let inline_key = credentials
        .oci_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret)
        .filter(|value| !value.is_empty());
    let key_file = credentials
        .oci_key_file
        .as_deref()
        .filter(|value| !value.is_empty());
    let (Some(user), Some(fingerprint), Some(tenancy)) =
        (user_value, fingerprint_value, tenancy_value)
    else {
        return Err(missing_credentials());
    };
    let key_text = match (inline_key, key_file) {
        (Some(key), _) => key.replace("\\n", "\n").replace("\r\n", "\n"),
        (None, Some(path)) => read_key_file(path)?,
        (None, None) => return Err(missing_credentials()),
    };
    Ok(SigningKey {
        key_id: format!("{tenancy}/{user}/{fingerprint}"),
        key_pair: parse_pem_key(&key_text)?,
    })
}

fn missing_credentials() -> OciError {
    OciError {
        status: 401,
        message: MISSING_CREDENTIALS.to_owned(),
    }
}

fn read_key_file(path: &str) -> Result<String, OciError> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Err(OciError {
                status: 500,
                message: format!("Private key file not found: {path}"),
            });
        }
        Err(source) => {
            return Err(OciError {
                status: 500,
                message: format!("Failed to read private key file '{path}': {source}"),
            });
        }
    };
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(OciError {
            status: 500,
            message: format!("Private key file is empty: {path}"),
        });
    }
    Ok(trimmed.to_owned())
}

fn parse_pem_key(text: &str) -> Result<KeyPair, OciError> {
    let block = pem::parse(text).map_err(|source| OciError {
        status: 500,
        message: source.to_string(),
    })?;
    let parsed = match block.tag() {
        "PRIVATE KEY" => KeyPair::from_pkcs8(block.contents()),
        "RSA PRIVATE KEY" => KeyPair::from_der(block.contents()),
        _ => {
            return Err(OciError {
                status: 500,
                message: NOT_RSA.to_owned(),
            });
        }
    };
    parsed.map_err(|source| OciError {
        status: 500,
        message: source.to_string(),
    })
}

struct SignatureSubject<'values> {
    date: &'values str,
    host: &'values str,
    path: &'values str,
    content_type: &'values str,
    content_length: &'values str,
    content_sha256: &'values str,
}

fn signing_string(subject: &SignatureSubject<'_>) -> String {
    format!(
        "date: {}\n(request-target): post {}\nhost: {}\ncontent-length: {}\ncontent-type: {}\nx-content-sha256: {}",
        subject.date,
        subject.path,
        subject.host,
        subject.content_length,
        subject.content_type,
        subject.content_sha256
    )
}

fn sign_string(key_pair: &KeyPair, signing_string: &str) -> Result<Vec<u8>, OciError> {
    let mut signature = vec![0_u8; key_pair.public_modulus_len()];
    key_pair
        .sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            signing_string.as_bytes(),
            &mut signature,
        )
        .map_err(|source| OciError {
            status: 500,
            message: format!("the OCI request could not be signed: {source}"),
        })?;
    Ok(signature)
}

fn authorization(key_id: &str, signature: &[u8]) -> String {
    format!(
        "Signature version=\"1\",keyId=\"{key_id}\",algorithm=\"rsa-sha256\",headers=\"{SIGNED_HEADER_LIST}\",signature=\"{}\"",
        STANDARD.encode(signature)
    )
}

struct SignedHeaders {
    authorization: String,
    date: String,
    host: String,
    content_type: String,
    content_length: String,
    content_sha256: String,
}

fn insert_signed_headers(headers: &mut HeaderMap, signed: &SignedHeaders) -> Result<(), OciError> {
    headers.insert(AUTHORIZATION, header_value(&signed.authorization)?);
    headers.insert(DATE, header_value(&signed.date)?);
    headers.insert(HOST, header_value(&signed.host)?);
    headers.insert(CONTENT_TYPE, header_value(&signed.content_type)?);
    headers.insert(CONTENT_LENGTH, header_value(&signed.content_length)?);
    headers.insert(CONTENT_SHA256, header_value(&signed.content_sha256)?);
    Ok(())
}

fn content_type(headers: &HeaderMap) -> Result<String, OciError> {
    headers.get(CONTENT_TYPE).map_or_else(
        || Ok("application/json".to_owned()),
        |value| {
            value
                .to_str()
                .map(str::to_owned)
                .map_err(|source| OciError {
                    status: 500,
                    message: format!("the OCI content-type header is not valid text: {source}"),
                })
        },
    )
}

fn header_value(text: &str) -> Result<HeaderValue, OciError> {
    HeaderValue::from_str(text).map_err(|source| OciError {
        status: 500,
        message: format!("a signed OCI header value is not valid: {source}"),
    })
}
