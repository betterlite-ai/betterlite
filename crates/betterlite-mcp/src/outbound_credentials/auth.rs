use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use crate::credential_contracts::errors::CredResult;
use crate::credential_contracts::header_carrier::{HeaderCarrier, validate_header_name};
use crate::credential_contracts::httpx_auth::NoOpAuth;
use crate::credentials::{CredentialError, Request, ResolvedAuth};

use super::collaborators::{ClientCredentialsSource, CredentialRequest};

impl ResolvedAuth for NoOpAuth {
    fn authenticate(&self, request: Request) -> Result<Request, CredentialError> {
        Ok(request)
    }

    fn close(&self) {}
}

pub(super) struct StaticAuth {
    header_name: String,
    value: SecretString,
}

impl StaticAuth {
    pub(super) fn new(header_name: &str, value: &str) -> CredResult<Self> {
        let validated = validate_header_name(header_name)?;
        Ok(Self {
            header_name: validated,
            value: SecretString::from(value.to_owned()),
        })
    }
}

pub(super) fn static_auth(
    header: &HeaderCarrier,
    value: &str,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let (header_name, header_value) = header.header(value);
    let auth = StaticAuth::new(&header_name, &header_value)?;
    Ok(Arc::new(auth))
}

impl ResolvedAuth for StaticAuth {
    fn authenticate(&self, mut request: Request) -> Result<Request, CredentialError> {
        let name = self.header_name.as_str();
        let value = self.value.expose_secret();
        insert_header(&mut request.headers, name, value);
        Ok(request)
    }

    fn close(&self) {}

    fn header_name(&self) -> Option<&str> {
        Some(&self.header_name)
    }
}

pub(super) struct ClientCredentialsAuth {
    source: Arc<dyn ClientCredentialsSource>,
    server_id: String,
    config: ClientCredentialsConfig,
    header_name: String,
    token: Mutex<SecretString>,
}

impl ClientCredentialsAuth {
    pub(super) fn new(
        source: Arc<dyn ClientCredentialsSource>,
        server_id: String,
        config: ClientCredentialsConfig,
        access_token: SecretString,
    ) -> Self {
        let (header_name, _formatted) = config.header.header("");
        Self {
            source,
            server_id,
            config,
            header_name,
            token: Mutex::new(access_token),
        }
    }
}

impl ResolvedAuth for ClientCredentialsAuth {
    fn authenticate(&self, mut request: Request) -> Result<Request, CredentialError> {
        let (header_name, header_value) = {
            let guard = self.token.lock().unwrap_or_else(PoisonError::into_inner);
            self.config.header.header(guard.expose_secret())
        };
        insert_header(&mut request.headers, &header_name, &header_value);
        Ok(request)
    }

    fn close(&self) {}

    fn header_name(&self) -> Option<&str> {
        Some(&self.header_name)
    }

    // The transport resends a 401 once when a fresh token replaced the rejected one, and leaves the upstream error standing otherwise.
    fn after_unauthorized(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move {
            let failed = {
                let guard = self.token.lock().unwrap_or_else(PoisonError::into_inner);
                SecretString::clone(&guard)
            };
            let request = CredentialRequest {
                server_id: &self.server_id,
                config: &self.config,
            };
            let refetched = self.source.refetch(request, &failed).await;
            refetched.is_some_and(|fresh| {
                let mut guard = self.token.lock().unwrap_or_else(PoisonError::into_inner);
                *guard = fresh;
                true
            })
        })
    }
}

// HTTP header names are case-insensitive, so a credential replaces any header already occupying its slot instead of adding a second one.
fn insert_header(headers: &mut BTreeMap<String, String>, name: &str, value: &str) {
    headers.retain(|existing, _value| !existing.eq_ignore_ascii_case(name));
    headers.insert(name.to_owned(), value.to_owned());
}
