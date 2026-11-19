use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_client_secret::SecretResolver;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::endpoints::projection::CallableProvider;
use betterlite_azure::oidc_exchange::transport::{Clock, HttpReply, Transport};
use betterlite_azure::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};
use betterlite_config::sections::azure_oidc_exchange::Settings;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::ExchangeSpec;
use super::FormView;

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> f64 {
        1_700_000_000.0
    }
}

struct RecordingTransport {
    status: u16,
    body: String,
    forms: Arc<Mutex<Vec<FormView>>>,
}

impl Transport for RecordingTransport {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        let fields: BTreeMap<String, String> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.forms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FormView {
                url: url.to_owned(),
                form: fields,
            });
        Ok(HttpReply {
            status: self.status,
            body: SecretString::from(self.body.clone()),
        })
    }
}

struct FixedResolver {
    assertion: SecretString,
}

impl SecretResolver for FixedResolver {
    fn get_secret_str(&self, _reference: &str) -> Option<SecretString> {
        if self.assertion.expose_secret().is_empty() {
            return None;
        }
        Some(self.assertion.clone())
    }
}

pub(crate) struct ScriptedSources {
    exchange: AzureOidcExchange,
    resolver: FixedResolver,
    pub(crate) forms: Arc<Mutex<Vec<FormView>>>,
}

impl ScriptedSources {
    pub(crate) fn new(spec: &ExchangeSpec) -> Self {
        let forms = Arc::new(Mutex::new(Vec::new()));
        let exchange = AzureOidcExchange::new(
            Settings::default(),
            spec.environment.clone(),
            Arc::new(RecordingTransport {
                status: spec.status,
                body: spec.body.clone(),
                forms: Arc::clone(&forms),
            }),
            Arc::new(FixedClock),
        )
        .expect("the OIDC exchange builds");
        ScriptedSources {
            exchange,
            resolver: FixedResolver {
                assertion: SecretString::from(spec.assertion.clone()),
            },
            forms,
        }
    }
}

fn unscripted() -> SourceFailure {
    SourceFailure {
        class: "CredentialSources".to_owned(),
        status: None,
        message: "not scripted".to_owned(),
    }
}

impl CredentialSources for ScriptedSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.exchange
            .token(input, &self.resolver)
            .map_err(|error| SourceFailure {
                class: "AzureOpenAIError".to_owned(),
                status: error.status_code(),
                message: error.to_string(),
            })
    }
}

struct DummyProvider;

impl CallableTokenProvider for DummyProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Err(ProviderFailure {
            class: "Dummy".to_owned(),
            message: "not scripted".to_owned(),
        })
    }
}

pub(crate) fn callable_provider(identity: u64) -> CallableProvider {
    CallableProvider {
        provider: Arc::new(DummyProvider),
        identity,
    }
}
