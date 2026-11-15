use std::sync::Arc;
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

// every grant answers with its arguments, so one token carries the source and the whole call; the Python side builds the same string
pub(super) struct EchoSources;

struct EchoProvider {
    token: String,
}

impl CallableTokenProvider for EchoProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from(self.token.clone())))
    }
}

fn oidc_token(input: &OidcExchangeInput) -> String {
    let client = input.azure_client_id.as_deref().unwrap_or("None");
    let tenant = input.azure_tenant_id.as_deref().unwrap_or("None");
    let scope = input.scope.as_deref().unwrap_or("None");
    format!(
        "oidc assertion={} client={client} tenant={tenant} scope={scope}",
        input.azure_ad_token.expose_secret()
    )
}

impl CredentialSources for EchoSources {
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let tenant_id = record.tenant_id.as_str();
        let client_id = record.client_id.as_str();
        let secret = record.client_secret.expose_secret();
        let scope = record.scope.as_str();
        let token = format!(
            "client_secret tenant={tenant_id} client={client_id} secret={secret} scope={scope}"
        );
        Ok(Arc::new(EchoProvider { token }))
    }

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let secret = record.password.expose_secret();
        let fields = [
            String::from("username_password"),
            format!("client={}", record.client_id),
            format!("user={}", record.username),
            format!("password={secret}"),
            format!("scope={}", record.scope),
        ];
        Ok(Arc::new(EchoProvider {
            token: fields.join(" "),
        }))
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Ok(Arc::new(EchoProvider {
            token: format!("sdk_refresh scope={scope}"),
        }))
    }

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        Some(Arc::new(EchoProvider {
            token: format!("default_credential scope={scope}"),
        }))
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Ok(SecretString::from(oidc_token(input)))
    }
}

struct Offline;

impl OidcHttp for Offline {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the oracle helper performs no HTTP requests".to_owned(),
        })
    }
}

impl AzureOidcTokens for Offline {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for Offline {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    let oidc_http: Arc<dyn OidcHttp> = Arc::new(Offline);
    let azure_tokens: Arc<dyn AzureOidcTokens> = Arc::new(Offline);
    let clock: Arc<dyn OidcClock> = Arc::new(Offline);
    SecretLookup {
        manager: None,
        oidc_http,
        azure_tokens,
        clock,
        oidc_cache: OidcTokenCache::default(),
    }
}
