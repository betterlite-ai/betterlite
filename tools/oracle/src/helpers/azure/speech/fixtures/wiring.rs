use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{ClientIdentity, ClientKind};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_azure::speech::binary::{AudioSpeechCall, audio_speech};
use betterlite_azure::speech::client::ClientWiring;
use betterlite_azure::speech::error::AzureSpeechError;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::proxy::outbound::OutboundRequest;
use secrecy::SecretString;

use super::handler::Recorder;

// no credential source resolves anything: a case carries its own api_key
#[derive(Default)]
pub(crate) struct NoSources;

fn no_source() -> SourceFailure {
    SourceFailure {
        class: String::from("RuntimeError"),
        status: None,
        message: String::from("no credential source behind the speech oracle case"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(no_source())
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    entries: HashMap<ClientIdentity, ClientKind>,
}

impl ClientCache for Cache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.entries.get(identity).copied()
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.entries.insert(identity, kind);
    }
}

#[derive(Default)]
pub(crate) struct Inputs {
    pub(crate) settings: Settings,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) sources: NoSources,
    pub(crate) cache: Cache,
}

pub(crate) fn start(
    inputs: &mut Inputs,
    call: &AudioSpeechCall<'_>,
    logger: &mut Recorder,
) -> Result<OutboundRequest, AzureSpeechError> {
    let mut refresh = RefreshProviderCache::new(&inputs.sources);
    let mut wiring = ClientWiring {
        settings: &inputs.settings,
        environment: &inputs.environment,
        sources: &inputs.sources,
        cache: &mut inputs.cache,
        refresh: &mut refresh,
    };
    audio_speech(call, &mut wiring, logger)
}
