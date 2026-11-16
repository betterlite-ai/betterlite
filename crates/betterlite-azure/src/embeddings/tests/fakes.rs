use std::collections::HashMap;
use std::sync::Arc;

use secrecy::SecretString;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::endpoints::get_client::ClientCache;
use crate::endpoints::projection::{ClientIdentity, ClientKind};
use crate::oidc_exchange::OidcExchangeInput;

#[derive(Default)]
pub(super) struct Cache(HashMap<ClientIdentity, ClientKind>);

impl ClientCache for Cache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.0.get(identity).copied()
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.0.insert(identity, kind);
    }
}

// every source fails; the status is what the SDK attached to its exception
#[derive(Default)]
pub(super) struct NoSources {
    pub(super) status: Option<u16>,
}

impl NoSources {
    fn failure(&self) -> SourceFailure {
        SourceFailure {
            class: "RuntimeError".to_owned(),
            status: self.status,
            message: "no credential source in this fixture".to_owned(),
        }
    }
}

type Provider<'source> = Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

impl CredentialSources for NoSources {
    fn client_secret(&self, _record: &ClientSecretRecord) -> Provider<'_> {
        Err(self.failure())
    }

    fn username_password(&self, _record: &UsernamePasswordRecord) -> Provider<'_> {
        Err(self.failure())
    }

    fn sdk_refresh(&self, _scope: &str) -> Provider<'_> {
        Err(self.failure())
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(self.failure())
    }
}
