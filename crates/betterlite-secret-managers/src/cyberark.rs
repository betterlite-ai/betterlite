use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::Duration;

use secrecy::SecretString;
use tokio::sync::Mutex;

use crate::secret_contracts::options::{DeleteOptions, SecretOptions, SecretRotation, SecretWrite};
use crate::secret_contracts::{BaseSecretManager, RotationError, SecretFuture};
use crate::secret_helpers::read_boolean_text;
use cyberark_auth::{TOKEN_KEY, begin_login, finish_login, login_request};
use cyberark_read::{finish_read, read_request, secret_url};
use error::{CyberArkError, CyberArkInitError};
use operation::SecretOperation;
use transport::{ClientCertificate, ConjurResponse, ConjurTransport};
use ttl_cache::TtlCache;

mod cyberark_auth;
mod cyberark_delete;
mod cyberark_read;
mod cyberark_write;
pub mod error;
mod fake_transport;
pub mod operation;
pub mod transport;
mod ttl_cache;
mod yaml_scalar;

const DEFAULT_REFRESH_SECONDS: u64 = 300;

#[derive(Debug)]
pub struct CyberArkSettings {
    pub api_base: String,
    pub account: String,
    pub username: String,
    pub api_key: SecretString,
    pub client_certificate: Option<ClientCertificate>,
    pub verify_tls: bool,
    pub refresh_interval: Duration,
}

impl CyberArkSettings {
    pub fn from_environment(
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, CyberArkInitError> {
        let lookup = |name: &str| environment.get(name).filter(|value| !value.is_empty());
        let or_default = |name: &str, default: &str| {
            lookup(name).map_or_else(|| default.to_owned(), Clone::clone)
        };
        let api_key = lookup("CYBERARK_API_KEY").map(|key| SecretString::from(key.as_str()));
        let client_certificate = lookup("CYBERARK_CLIENT_CERT")
            .zip(lookup("CYBERARK_CLIENT_KEY"))
            .map(|(certificate, key)| ClientCertificate {
                certificate_path: certificate.clone(),
                key_path: key.clone(),
            });
        if api_key.is_none() && client_certificate.is_none() {
            return Err(CyberArkInitError::MissingCredentials);
        }
        let refresh_seconds = lookup("CYBERARK_REFRESH_INTERVAL")
            .map(|text| betterlite_env::number::<u64>("CYBERARK_REFRESH_INTERVAL", text))
            .transpose()?
            .flatten()
            .unwrap_or(DEFAULT_REFRESH_SECONDS);
        let verify_text = lookup("CYBERARK_SSL_VERIFY").map(String::as_str);
        Ok(Self {
            api_base: or_default("CYBERARK_API_BASE", "http://127.0.0.1:8080"),
            account: or_default("CYBERARK_ACCOUNT", "default"),
            username: or_default("CYBERARK_USERNAME", "admin"),
            api_key: api_key.unwrap_or_else(|| SecretString::from("")),
            client_certificate,
            verify_tls: read_boolean_text("CYBERARK_SSL_VERIFY", verify_text)
                .ok()
                .flatten()
                .unwrap_or(true),
            refresh_interval: Duration::from_secs(refresh_seconds),
        })
    }
}

pub struct CyberArkSecretManager<T> {
    settings: CyberArkSettings,
    transport: T,
    values: TtlCache,
    tokens: TtlCache,
    policy_load_lock: Mutex<()>,
}

impl<T: ConjurTransport> CyberArkSecretManager<T> {
    pub fn new(
        environment: &BTreeMap<String, String>,
        transport: T,
    ) -> Result<Self, CyberArkInitError> {
        let settings = CyberArkSettings::from_environment(environment)?;
        if !settings.verify_tls {
            tracing::warn!(
                "CyberArk SSL verification is disabled. This is insecure and should only be used for testing with self-signed certificates."
            );
        }
        Ok(Self {
            values: TtlCache::new(settings.refresh_interval),
            tokens: TtlCache::new(settings.refresh_interval),
            policy_load_lock: Mutex::new(()),
            settings,
            transport,
        })
    }

    async fn authorization(&self) -> Result<SecretString, CyberArkError<T::Error>> {
        if let Some(header) = self.tokens.get(TOKEN_KEY) {
            return Ok(header);
        }
        let url = begin_login(self);
        finish_login(self, self.transport.send(login_request(self, &url)).await)
    }

    fn authorization_blocking(&self) -> Result<SecretString, CyberArkError<T::Error>> {
        if let Some(header) = self.tokens.get(TOKEN_KEY) {
            return Ok(header);
        }
        let url = begin_login(self);
        finish_login(
            self,
            self.transport.send_blocking(login_request(self, &url)),
        )
    }

    async fn read(&self, secret_name: &str) -> Option<SecretString> {
        if let Some(cached) = self.values.get(secret_name) {
            return Some(cached);
        }
        let outcome = self.fetch(secret_name).await;
        finish_read(self, secret_name, outcome)
    }

    fn read_blocking(&self, secret_name: &str) -> Option<SecretString> {
        if let Some(cached) = self.values.get(secret_name) {
            return Some(cached);
        }
        let outcome = self.fetch_blocking(secret_name);
        finish_read(self, secret_name, outcome)
    }

    async fn fetch(&self, secret_name: &str) -> Result<ConjurResponse, CyberArkError<T::Error>> {
        let header = self.authorization().await?;
        let url = secret_url(self, secret_name);
        let reply = self.transport.send(read_request(self, &url, &header)).await;
        reply.map_err(CyberArkError::Transport)?.require_success()
    }

    fn fetch_blocking(&self, secret_name: &str) -> Result<ConjurResponse, CyberArkError<T::Error>> {
        let header = self.authorization_blocking()?;
        let url = secret_url(self, secret_name);
        let reply = self
            .transport
            .send_blocking(read_request(self, &url, &header));
        reply.map_err(CyberArkError::Transport)?.require_success()
    }
}

impl<T: ConjurTransport> BaseSecretManager for CyberArkSecretManager<T> {
    type Params = ();
    type Response = SecretOperation;
    type Error = Infallible;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        _options: SecretOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, Self::Error> {
        Box::pin(async move { Ok(self.read(secret_name).await) })
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        _options: SecretOptions<'_, Self::Params>,
    ) -> Result<Option<SecretString>, Self::Error> {
        Ok(self.read_blocking(secret_name))
    }

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        write: SecretWrite<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error> {
        Box::pin(async move {
            Ok(cyberark_write::write(self, write.secret_name, write.secret_value).await)
        })
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        _options: DeleteOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error> {
        Box::pin(async move { Ok(cyberark_delete::delete(self, secret_name)) })
    }

    fn async_rotate_secret<'borrowed>(
        &'borrowed self,
        rotation: SecretRotation<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, RotationError<Self::Error>> {
        Box::pin(cyberark_delete::rotate(self, rotation))
    }
}
