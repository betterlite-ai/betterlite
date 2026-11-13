use std::fmt;
use std::fs;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::{ExposeSecret as _, SecretSlice, SecretString};

use super::CertificateError;
use super::assertion::{Assertions, IdSource};
use super::exchange::{assertion_request, read_token_reply};
use super::material::{CertificateMaterial, decode_certificate_material};
use crate::azure_client_secret::authority;
use crate::azure_client_secret::token::AccessToken;
use crate::azure_client_secret::transport::{Clock, Transport};
use crate::azure_client_secret::{ClientSecretError, DEFAULT_AUTHORITY_HOST, DEFAULT_SCOPE};

// the certificate file is read through this seam so a fixture can stand in for the disk
pub trait FileReader: Send + Sync {
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
}

pub struct StdFiles;

impl FileReader for StdFiles {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}

// no Debug: certificate_data holds the private key
pub struct CertificateInput {
    pub tenant_id: String,
    pub client_id: String,
    pub scope: Option<String>,
    pub certificate_path: Option<String>,
    pub certificate_data: Option<SecretSlice<u8>>,
    pub password: Option<SecretString>,
    pub send_certificate_chain: bool,
    pub authority_host: Option<String>,
}

#[derive(Default)]
struct State {
    token: Option<AccessToken>,
    token_endpoint: Option<String>,
}

pub struct CertificateCredential {
    tenant_id: String,
    client_id: String,
    scope: String,
    authority_host: String,
    assertions: Assertions,
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

// the private key, its assertions and every acquired token stay out of the Debug output
impl fmt::Debug for CertificateCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CertificateCredential")
            .field("tenant_id", &self.tenant_id)
            .field("client_id", &self.client_id)
            .field("scope", &self.scope)
            .field("authority_host", &self.authority_host)
            .finish_non_exhaustive()
    }
}

impl CertificateCredential {
    // azure-identity checks the tenant first, then the certificate source, then msal normalizes the authority
    pub fn new(
        input: CertificateInput,
        files: &dyn FileReader,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdSource>,
    ) -> Result<Self, CertificateError> {
        let valid = !input.tenant_id.is_empty()
            && input.tenant_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '.')
            });
        if !valid {
            return Err(CertificateError::Client(ClientSecretError::InvalidTenantId));
        }
        let material = load_material(&input, files)?;
        let authority_host = authority::normalize_authority(
            input
                .authority_host
                .as_deref()
                .unwrap_or(DEFAULT_AUTHORITY_HOST),
        )?;
        Ok(Self {
            tenant_id: input.tenant_id,
            scope: input.scope.unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
            authority_host,
            assertions: Assertions {
                material,
                client_id: input.client_id.clone(),
                send_chain: input.send_certificate_chain,
                ids,
                reusable: Mutex::new(None),
            },
            client_id: input.client_id,
            transport,
            clock,
            state: Mutex::new(State::default()),
        })
    }

    pub fn token(&self) -> Result<AccessToken, CertificateError> {
        let now = self.clock.now_unix_seconds();
        let cached = self.state()?.token.clone();
        if let Some(token) = &cached
            && now < token.refresh_on.unwrap_or(token.expires_on)
        {
            return Ok(token.clone());
        }
        match self.acquire(now) {
            Ok(token) => {
                self.state()?.token = Some(token.clone());
                Ok(token)
            }
            // msal serves the aging cached token only when the IdP refuses the refresh; other failures raise
            Err(error @ CertificateError::Refused { .. }) => match cached {
                Some(token) if now < token.expires_on => Ok(token),
                _ => Err(error),
            },
            Err(error) => Err(error),
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, ClientSecretError> {
        self.state
            .lock()
            .map_err(|_error| ClientSecretError::ProviderCachePoisoned)
    }

    // msal builds its application, and so resolves the authority, once per credential
    fn acquire(&self, now: i64) -> Result<AccessToken, CertificateError> {
        let known = self.state()?.token_endpoint.clone();
        let token_endpoint = if let Some(endpoint) = known {
            endpoint
        } else {
            let found = authority::discover_authority(
                &*self.transport,
                &self.authority_host,
                &self.tenant_id,
                true,
            )?;
            self.state()?.token_endpoint = Some(found.token_endpoint.clone());
            found.token_endpoint
        };
        let assertion = self.assertions.assertion(&token_endpoint, now)?;
        let request = assertion_request(&token_endpoint, &self.client_id, &self.scope, &assertion);
        let reply = self
            .transport
            .send(&request)
            .map_err(|source| ClientSecretError::TokenTransport { source })?;
        read_token_reply(&reply, now)
    }
}

fn load_material(
    input: &CertificateInput,
    files: &dyn FileReader,
) -> Result<CertificateMaterial, CertificateError> {
    let configured_path = input
        .certificate_path
        .as_deref()
        .filter(|text| !text.is_empty());
    let configured_data = input
        .certificate_data
        .as_ref()
        .filter(|secret| !secret.expose_secret().is_empty());
    let bytes = match (configured_path, configured_data) {
        (Some(_), Some(_)) => return Err(CertificateError::BothSources),
        (Some(file), None) => {
            let read = files.read(file).map_err(|source| CertificateError::Read {
                path: file.to_owned(),
                source,
            })?;
            SecretSlice::from(read)
        }
        (None, Some(inline)) => inline.clone(),
        (None, None) => return Err(CertificateError::NoSource),
    };
    decode_certificate_material(bytes.expose_secret(), input.password.as_ref())
}
