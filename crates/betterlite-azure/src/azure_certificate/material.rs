use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p12_keystore::{Certificate, Pkcs12Archive};
use pkcs8::der::asn1::ObjectIdentifier;
use pkcs8::{EncryptedPrivateKeyInfoRef, Error as Pkcs8Error, PrivateKeyInfoRef};
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use sha1::{Digest as _, Sha1};

use super::{CertificateError, DecodeFailure};

const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const PEM_MARKER: &[u8] = b"-----BEGIN";

type Rsa = Option<SecretSlice<u8>>;

// the key is PKCS#1 DER, the signing certificate comes first in the chain, and the thumbprint is its SHA-1 fingerprint
#[derive(Debug)]
pub struct CertificateMaterial {
    pub key: SecretSlice<u8>,
    pub thumbprint: [u8; 20],
    pub chain: Vec<Vec<u8>>,
}

pub fn decode_certificate_material(
    data: &[u8],
    password: Option<&SecretString>,
) -> Result<CertificateMaterial, CertificateError> {
    let password_text = password
        .map(SecretString::expose_secret)
        .filter(|text| !text.is_empty());
    let (rsa, chain) = if data
        .windows(PEM_MARKER.len())
        .any(|window| window == PEM_MARKER)
    {
        pem_parts(data, password_text)?
    } else {
        pkcs12_parts(data, password_text)?
    };
    let key = rsa.ok_or(CertificateError::NotRsa)?;
    let signing = chain
        .first()
        .ok_or(CertificateError::NoSigningCertificate)?;
    Certificate::from_der(signing).map_err(|source| CertificateError::Deserialize {
        source: DecodeFailure::from(source),
    })?;
    let thumbprint = <[u8; 20]>::from(Sha1::digest(signing));
    // cryptography rejects a key it cannot use when it loads the file, so a trial signature does the same
    let trial = EncodingKey::from_rsa_der(key.expose_secret());
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &(), &trial)
        .map_err(|source| CertificateError::Sign { source })?;
    Ok(CertificateMaterial {
        key,
        thumbprint,
        chain,
    })
}

fn rsa_from_pkcs8(der: &[u8]) -> Result<Rsa, Pkcs8Error> {
    let info = PrivateKeyInfoRef::try_from(der)?;
    Ok((info.algorithm.oid == RSA_ENCRYPTION)
        .then(|| SecretSlice::from(info.private_key.as_bytes().to_vec())))
}

fn pem_key(block: &pem::Pem, password: Option<&str>) -> Result<Rsa, CertificateError> {
    let der = block.contents();
    let legacy_encrypted = block.headers().get("Proc-Type").is_some();
    match (block.tag(), password, legacy_encrypted) {
        ("ENCRYPTED PRIVATE KEY", None, _) | ("RSA PRIVATE KEY", None, true) => {
            Err(CertificateError::PasswordRequired)
        }
        ("ENCRYPTED PRIVATE KEY", Some(secret), _) => {
            let info = EncryptedPrivateKeyInfoRef::try_from(der)
                .map_err(|source| CertificateError::KeyData { source })?;
            let plain = info
                .decrypt(secret)
                .map_err(|source| CertificateError::BadPassword { source })?;
            rsa_from_pkcs8(plain.as_bytes()).map_err(|source| CertificateError::KeyData { source })
        }
        ("RSA PRIVATE KEY", Some(_), true) => Err(CertificateError::LegacyEncryptedPem),
        (_, Some(_), _) => Err(CertificateError::PasswordUnexpected),
        ("PRIVATE KEY", None, _) => {
            rsa_from_pkcs8(der).map_err(|source| CertificateError::KeyData { source })
        }
        ("RSA PRIVATE KEY", None, false) => Ok(Some(SecretSlice::from(der.to_vec()))),
        ("EC PRIVATE KEY" | "DSA PRIVATE KEY", None, _) => Ok(None),
        _ => Err(CertificateError::UnsupportedKeyBlock),
    }
}

fn pem_parts(data: &[u8], password: Option<&str>) -> Result<(Rsa, Vec<Vec<u8>>), CertificateError> {
    let blocks = pem::parse_many(data).map_err(|source| CertificateError::PemFraming { source })?;
    if blocks.is_empty() {
        return Err(CertificateError::NoPemBlock);
    }
    let block = blocks
        .iter()
        .find(|block| block.tag().ends_with("PRIVATE KEY"))
        .ok_or(CertificateError::NoPrivateKeyBlock)?;
    Ok((pem_key(block, password)?, pem_chain(&blocks)?))
}

fn pem_chain(blocks: &[pem::Pem]) -> Result<Vec<Vec<u8>>, CertificateError> {
    let chain: Vec<Vec<u8>> = blocks
        .iter()
        .filter(|candidate| candidate.tag() == "CERTIFICATE")
        .map(|candidate| candidate.contents().to_vec())
        .collect();
    if chain.is_empty() {
        return Err(CertificateError::NoCertificateBlock);
    }
    Ok(chain)
}

fn pkcs12_parts(
    data: &[u8],
    password: Option<&str>,
) -> Result<(Rsa, Vec<Vec<u8>>), CertificateError> {
    let archive =
        Pkcs12Archive::from_pkcs12(data, password.unwrap_or_default()).map_err(|source| {
            CertificateError::Deserialize {
                source: DecodeFailure::from(source),
            }
        })?;
    let key = archive
        .keys
        .first()
        .ok_or(CertificateError::MissingPrivateKey)?;
    let wanted = key.local_key_id.as_ref().map(|id| id.0.as_slice());
    let signing = archive
        .certs
        .iter()
        .position(|bag| bag.local_key_id.as_deref() == wanted)
        .ok_or(CertificateError::NoSigningCertificate)?;
    let mut chain: Vec<Vec<u8>> = archive
        .certs
        .iter()
        .map(|bag| bag.cert.as_der().to_vec())
        .collect();
    let leaf = chain.remove(signing);
    chain.insert(0, leaf);
    let rsa = rsa_from_pkcs8(key.key.as_der()).map_err(|source| CertificateError::Deserialize {
        source: DecodeFailure::from(source),
    })?;
    Ok((rsa, chain))
}
