use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{ClientIdentity, ClientKind};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use secrecy::SecretString;

fn unavailable() -> SourceFailure {
    SourceFailure {
        class: "RuntimeError".to_owned(),
        status: None,
        message: "the fixture has no credential source".to_owned(),
    }
}

type Provider<'source> = Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

pub(crate) struct Sources;

impl CredentialSources for Sources {
    fn client_secret(&self, _record: &ClientSecretRecord) -> Provider<'_> {
        Err(unavailable())
    }

    fn username_password(&self, _record: &UsernamePasswordRecord) -> Provider<'_> {
        Err(unavailable())
    }

    fn sdk_refresh(&self, _scope: &str) -> Provider<'_> {
        Err(unavailable())
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable())
    }
}

pub(crate) struct Cache;

impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}
