use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Instant;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::engine::error::EngineError;
use secrecy::SecretString;

// the fixture credentials are the aws keys the call resolved from its options and environment
pub(crate) struct Source;

impl CredentialSource for Source {
    fn resolve(
        &self,
        settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(Credentials {
            access_key_id: settings
                .access_key_id
                .unwrap_or_else(|| SecretString::from(String::new())),
            secret_access_key: settings
                .secret_access_key
                .unwrap_or_else(|| SecretString::from(String::new())),
            session_token: settings.session_token,
        }))
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC transport"),
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

pub(crate) fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

// LiteLLM counts a text's tokens with the caller's encoding; the fixture counts characters
pub(crate) struct Counting;

impl TextEncoding for Counting {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
    }
}
