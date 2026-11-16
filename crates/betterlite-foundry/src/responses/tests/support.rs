use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::model_info::ModelInfoAccessors;
use crate::responses::auth::{EnvironmentRequest, validate_environment};

#[derive(Default)]
pub(super) struct FixtureSources {
    pub(super) calls: Mutex<Vec<String>>,
}

pub(super) struct FailingSources;

struct EchoProvider {
    token: String,
}

impl CallableTokenProvider for EchoProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from(self.token.clone())))
    }
}

fn offline() -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: String::from("provider refused"),
    }
}

// the client-secret grant answers with its arguments, so one token carries the whole identity
impl CredentialSources for FixtureSources {
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let token = format!(
            "client_secret tenant={} client={} secret={} scope={}",
            record.tenant_id.as_str(),
            record.client_id.as_str(),
            record.client_secret.expose_secret(),
            record.scope.as_str(),
        );
        self.calls.lock().unwrap().push(token.clone());
        Ok(Arc::new(EchoProvider { token }))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(offline())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(offline())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(offline())
    }
}

impl CredentialSources for FailingSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(offline())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(offline())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(offline())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(offline())
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn object<const N: usize>(entries: [(&str, Json); N]) -> Object {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) const fn no_accessors() -> ModelInfoAccessors<'static> {
    ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    }
}

// the caller headers, model-info layers and auth inputs vary per case; everything else is the empty fixture
pub(super) fn headers_for(
    caller: &BTreeMap<String, String>,
    accessors: ModelInfoAccessors<'_>,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> BTreeMap<String, String> {
    let env = environment(&[]);
    let sources = FixtureSources::default();
    validate_environment(&EnvironmentRequest {
        headers: caller,
        accessors,
        api_base,
        api_key,
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap()
}
