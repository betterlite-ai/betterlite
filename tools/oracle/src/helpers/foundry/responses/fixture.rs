use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use secrecy::{ExposeSecret as _, SecretString};

// each grant answers with its arguments, so one token carries the whole identity; the Python side builds the same string
pub(super) struct EchoSources;

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
        message: String::from("the oracle helper performs no HTTP requests"),
    }
}

impl CredentialSources for EchoSources {
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
        Ok(Arc::new(EchoProvider { token }))
    }

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let fields = [
            String::from("username_password"),
            format!("client={}", record.client_id),
            format!("user={}", record.username),
            format!("password={}", record.password.expose_secret()),
            format!("scope={}", record.scope),
        ];
        Ok(Arc::new(EchoProvider {
            token: fields.join(" "),
        }))
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

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        let client = input.azure_client_id.as_deref().unwrap_or("None");
        let tenant = input.azure_tenant_id.as_deref().unwrap_or("None");
        let scope = input.scope.as_deref().unwrap_or("None");
        Ok(SecretString::from(format!(
            "oidc assertion={} client={client} tenant={tenant} scope={scope}",
            input.azure_ad_token.expose_secret()
        )))
    }
}
