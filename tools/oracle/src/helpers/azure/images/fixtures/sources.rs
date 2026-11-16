use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use secrecy::SecretString;

type Source<'source> = Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

pub(crate) struct NoSources;

impl CredentialSources for NoSources {
    fn client_secret<'a>(&'a self, _record: &ClientSecretRecord) -> Source<'a> {
        Err(unavailable("client secret"))
    }

    fn username_password<'a>(&'a self, _record: &UsernamePasswordRecord) -> Source<'a> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'a>(&'a self, _scope: &str) -> Source<'a> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'a>(
        &'a self,
        _scope: &'a str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'a>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}
