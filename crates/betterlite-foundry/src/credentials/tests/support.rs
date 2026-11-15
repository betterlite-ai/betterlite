use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret as _, SecretString};

#[derive(Debug, Clone)]
pub(super) enum Outcome {
    Token(String),
    NonText(String),
    Raise { class: String, message: String },
}

pub(super) struct ScriptedProvider {
    outcome: Outcome,
}

impl CallableTokenProvider for ScriptedProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        match &self.outcome {
            Outcome::Token(text) => Ok(TokenReply::Text(SecretString::from(text.clone()))),
            Outcome::NonText(type_name) => Ok(TokenReply::NonText {
                type_name: type_name.clone(),
            }),
            Outcome::Raise { class, message } => Err(ProviderFailure {
                class: class.clone(),
                message: message.clone(),
            }),
        }
    }
}

pub(super) struct FixtureSources {
    pub(super) client_secret: Result<Outcome, SourceFailure>,
    pub(super) username_password: Result<Outcome, SourceFailure>,
    pub(super) oidc: Result<String, SourceFailure>,
    calls: Mutex<Vec<String>>,
}

impl FixtureSources {
    pub(super) fn new() -> Self {
        Self {
            client_secret: Ok(Outcome::Token("client-secret-token".to_owned())),
            username_password: Ok(Outcome::Token("username-token".to_owned())),
            oidc: Ok("oidc-token".to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn provider(
        &self,
        call: String,
        outcome: &Result<Outcome, SourceFailure>,
    ) -> Result<Arc<dyn CallableTokenProvider + '_>, SourceFailure> {
        self.record(call);
        match outcome {
            Ok(script) => Ok(Arc::new(ScriptedProvider {
                outcome: script.clone(),
            })),
            Err(failure) => Err(failure.clone()),
        }
    }
}

fn client_call(record: &ClientSecretRecord) -> String {
    let ClientSecretRecord {
        tenant_id,
        client_id,
        client_secret,
        scope,
    } = record;
    let secret = client_secret.expose_secret();
    format!("client_secret tenant={tenant_id} client={client_id} secret={secret} scope={scope}")
}

fn username_call(record: &UsernamePasswordRecord) -> String {
    let UsernamePasswordRecord {
        client_id,
        username,
        password,
        tenant_id,
        scope,
    } = record;
    let password = password.expose_secret();
    format!(
        "username_password client={client_id} user={username} password={password} scope={scope} tenant={tenant_id:?}"
    )
}

impl CredentialSources for FixtureSources {
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.provider(client_call(record), &self.client_secret)
    }

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.provider(username_call(record), &self.username_password)
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.record(format!("sdk_refresh {scope}"));
        Err(SourceFailure {
            class: "ValueError".to_owned(),
            status: None,
            message: "no refresh provider is configured".to_owned(),
        })
    }

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        self.record(format!("default_credential {scope}"));
        None
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.record(format!(
            "oidc assertion={} client={:?} tenant={:?} scope={:?}",
            input.azure_ad_token.expose_secret(),
            input.azure_client_id,
            input.azure_tenant_id,
            input.scope
        ));
        match &self.oidc {
            Ok(token) => Ok(SecretString::from(token.clone())),
            Err(failure) => Err(failure.clone()),
        }
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no HTTP transport in tests".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    let transport = Arc::new(NoOidc);
    SecretLookup {
        manager: None,
        oidc_http: Arc::clone(&transport),
        azure_tokens: Arc::clone(&transport),
        clock: transport,
        oidc_cache: OidcTokenCache::default(),
    }
}
