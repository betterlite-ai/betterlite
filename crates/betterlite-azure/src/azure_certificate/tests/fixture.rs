use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation};
use secrecy::{SecretSlice, SecretString};
use serde::Deserialize;

use crate::azure_certificate::CertificateError;
use crate::azure_certificate::assertion::IdSource;
use crate::azure_certificate::credential::{CertificateCredential, CertificateInput, FileReader};
use crate::azure_client_secret::transport::{Clock, HttpMethod, HttpReply, HttpRequest, Transport};

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const TENANT: &str = "tenant";
pub(super) const CLIENT: &str = "client";
pub(super) const PASSWORD: &str = "fixture-password";
pub(super) const TOKEN_URL: &str = "https://login.microsoftonline.com/tenant/oauth2/v2.0/token";
// SHA-1 of the fixture leaf certificate, as hex and as padded URL-safe base64
pub(super) const LEAF_SHA1: &str = "b3435e29060109386a04449e6c56e08a9c24af60";
pub(super) const LEAF_X5T: &str = "s0NeKQYBCThqBESebFbgipwkr2A=";
pub(super) const RSA_PKCS8: &[u8] = include_bytes!("fixtures/rsa_pkcs8.pem");
pub(super) const RSA_PKCS1: &[u8] = include_bytes!("fixtures/rsa_pkcs1.pem");
pub(super) const RSA_CHAIN: &[u8] = include_bytes!("fixtures/rsa_chain.pem");
pub(super) const RSA_ENCRYPTED: &[u8] = include_bytes!("fixtures/rsa_encrypted.pem");
pub(super) const RSA_NO_CERTIFICATE: &[u8] = include_bytes!("fixtures/rsa_no_certificate.pem");
pub(super) const CERTIFICATE_ONLY: &[u8] = include_bytes!("fixtures/certificate_only.pem");
pub(super) const RSA_PUBLIC: &[u8] = include_bytes!("fixtures/rsa_public.pem");
pub(super) const EC_PEM: &[u8] = include_bytes!("fixtures/ec.pem");
pub(super) const EC_P12: &[u8] = include_bytes!("fixtures/ec.p12");
pub(super) const RSA_P12: &[u8] = include_bytes!("fixtures/rsa.p12");
pub(super) const RSA_LEGACY_P12: &[u8] = include_bytes!("fixtures/rsa_legacy.p12");
pub(super) const RSA_UNPROTECTED_P12: &[u8] = include_bytes!("fixtures/rsa_unprotected.p12");
pub(super) const KEYLESS_P12: &[u8] = include_bytes!("fixtures/keyless.p12");

pub(super) fn openid(tenant: &str) -> (u16, String) {
    let base = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0");
    let body = format!(
        r#"{{"authorization_endpoint":"{base}/authorize","token_endpoint":"{base}/token"}}"#
    );
    (200, body)
}

pub(super) fn token_reply(body: &str) -> (u16, String) {
    (200, body.to_owned())
}

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<(u16, String)>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FixtureTransport {
    pub(super) fn new(replies: Vec<(u16, String)>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn posts(&self) -> Vec<HttpRequest> {
        let all = self.requests();
        all.into_iter()
            .filter(|request| request.method == HttpMethod::Post)
            .collect()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let (status, body) = next.ok_or_else(|| io::Error::other("no reply left"))?;
        Ok(HttpReply {
            status,
            body: SecretString::from(body),
        })
    }
}

pub(super) struct FixtureClock(pub(super) AtomicI64);

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub(super) struct CountingIds(pub(super) AtomicUsize);

impl IdSource for CountingIds {
    fn next_id(&self) -> String {
        format!("jti-{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}

#[derive(Default)]
pub(super) struct MapFiles {
    pub(super) files: BTreeMap<String, Vec<u8>>,
    pub(super) reads: Mutex<Vec<String>>,
}

impl FileReader for MapFiles {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.to_owned());
        let found = self.files.get(path);
        found
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }
}

pub(super) struct Fixture {
    pub(super) transport: Arc<FixtureTransport>,
    pub(super) clock: Arc<FixtureClock>,
    pub(super) files: MapFiles,
}

impl Fixture {
    pub(super) fn new(replies: Vec<(u16, String)>) -> Self {
        Self {
            transport: Arc::new(FixtureTransport::new(replies)),
            clock: Arc::new(FixtureClock(AtomicI64::new(NOW))),
            files: MapFiles::default(),
        }
    }

    pub(super) fn credential(
        &self,
        input: CertificateInput,
    ) -> Result<CertificateCredential, CertificateError> {
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        let ids = Arc::new(CountingIds(AtomicUsize::new(1)));
        CertificateCredential::new(input, &self.files, transport, clock, ids)
    }
}

pub(super) fn input(data: &[u8]) -> CertificateInput {
    CertificateInput {
        tenant_id: TENANT.to_owned(),
        client_id: CLIENT.to_owned(),
        scope: None,
        certificate_path: None,
        certificate_data: Some(SecretSlice::from(data.to_vec())),
        password: None,
        send_certificate_chain: false,
        authority_host: None,
    }
}

pub(super) fn with_password(mut base: CertificateInput, password: &str) -> CertificateInput {
    base.password = Some(SecretString::from(password));
    base
}

#[derive(Deserialize)]
pub(super) struct Claims {
    pub(super) aud: String,
    pub(super) iss: String,
    pub(super) sub: String,
    pub(super) exp: i64,
    pub(super) iat: i64,
    pub(super) jti: String,
    pub(super) nbf: Option<i64>,
}

// the signature is checked against the fixture's public key, not the key the credential holds
pub(super) fn decode(assertion: &str) -> (Header, Claims) {
    let public = pem::parse(RSA_PUBLIC).unwrap();
    let key = DecodingKey::from_rsa_der(public.contents());
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.required_spec_claims.clear();
    let decoded = jsonwebtoken::decode::<Claims>(assertion, &key, &validation).unwrap();
    (decoded.header, decoded.claims)
}

pub(super) fn form(request: &HttpRequest) -> BTreeMap<String, String> {
    request.form.iter().cloned().collect()
}
