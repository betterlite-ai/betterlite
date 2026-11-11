use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_azure::azure_certificate::assertion::IdSource;
use betterlite_azure::azure_client_secret::transport::Clock;
use betterlite_azure::azure_managed_identity::ManagedIdentityCredential;
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::imds::ProbePolicy;
use betterlite_azure::azure_managed_identity::runtime::{
    ImdsIo, ManagedIdentityIo, Scheduler, TokenFiles, WorkloadIo,
};
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_azure::oidc_exchange::transport::{Clock as OidcClock, Transport as OidcTransport};
use betterlite_config::sections::azure_oidc_exchange::Settings;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use self::transport::{Shared, SharedTransport};

pub(crate) mod transport;

const REQUEST_ID: &str = "00000000-0000-4000-8000-000000000001";
const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com";

pub(crate) struct FixedOidcClock {
    now: f64,
}

impl OidcClock for FixedOidcClock {
    fn now_unix_seconds(&self) -> f64 {
        self.now
    }
}

pub(crate) struct Fixture {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) files: BTreeMap<String, String>,
    pub(crate) now: i64,
    pub(crate) shared: Arc<Shared>,
    identity: ManagedIdentityEnvironment,
    transport: Arc<SharedTransport>,
    exchange: AzureOidcExchange,
    redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new(
        environment: &BTreeMap<String, String>,
        files: &BTreeMap<String, String>,
        now: i64,
        shared: Arc<Shared>,
    ) -> Result<Self, FixtureError> {
        let transport = Arc::new(SharedTransport {
            shared: Arc::clone(&shared),
        });
        let recorded: Arc<SharedTransport> = Arc::clone(&transport);
        let oidc_transport: Arc<dyn OidcTransport> = recorded;
        let now_seconds = f64::from(i32::try_from(now).map_err(|_error| FixtureError::Clock)?);
        let exchange = AzureOidcExchange::new(
            Settings {
                azure_authority_host: environment
                    .get("AZURE_AUTHORITY_HOST")
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_AUTHORITY.to_owned()),
            },
            environment.clone(),
            oidc_transport,
            Arc::new(FixedOidcClock { now: now_seconds }),
        )
        .map_err(FixtureError::Exchange)?;
        Ok(Self {
            environment: environment.clone(),
            files: files.clone(),
            now,
            shared,
            identity: ManagedIdentityEnvironment::from_lookup(environment),
            transport,
            exchange,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
        })
    }

    pub(crate) const fn credential<'fixture>(
        &'fixture self,
        client_id: Option<&'fixture str>,
        exclude_workload_identity: bool,
        probe: ProbePolicy,
    ) -> ManagedIdentityCredential<'fixture, Self> {
        ManagedIdentityCredential {
            environment: &self.identity,
            host: self,
            client_id,
            exclude_workload_identity,
            within_default_credential: false,
            probe,
        }
    }

    pub(crate) fn io(&self) -> ManagedIdentityIo<'_> {
        ManagedIdentityIo {
            imds: ImdsIo {
                transport: self.transport.as_ref(),
                scheduler: self,
                ids: self,
                clock: self,
            },
            hosted: betterlite_azure::azure_hosted_identity::request::HostedIo {
                transport: self.transport.as_ref(),
                clock: self,
                redactor: &self.redactor,
            },
            arc: self.transport.as_ref(),
            workload: WorkloadIo {
                exchange: &self.exchange,
                files: self,
            },
        }
    }
}

impl Scheduler for Fixture {
    fn sleep(&self, delay: Duration) {
        self.shared
            .sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay.as_secs_f64());
    }
}

impl Clock for Fixture {
    fn now_unix_seconds(&self) -> i64 {
        self.now
    }
}

impl IdSource for Fixture {
    fn next_id(&self) -> String {
        REQUEST_ID.to_owned()
    }
}

impl TokenFiles for Fixture {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.files
            .get(path)
            .map(|content| content.as_bytes().to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error(transparent)]
    Exchange(#[from] betterlite_azure::oidc_exchange::OidcExchangeError),
    #[error(transparent)]
    Redaction(#[from] RedactionError),
    #[error("the fixture clock is out of range")]
    Clock,
}
