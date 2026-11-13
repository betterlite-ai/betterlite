use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use betterlite_bedrock::aws_settings::credential_cache::CredentialCache;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_context::local_ttl_cache::dual::{CacheEvent, Diagnostics, DualLocalCache};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::Redactor;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

use crate::helpers::HelperError;

#[derive(Default)]
pub(super) struct ResolutionFixture {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl ResolutionFixture {
    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
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

pub(super) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "S3AccessFixture".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn credential_cache() -> Result<Arc<CredentialCache>, Box<HelperError>> {
    let memory = LocalTtlCache::with_clock(Some(1000), None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(failure)?;
    let emit = Arc::new(|_event: CacheEvent| {});
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(16).map_err(failure)?),
        emit,
    };
    let cache = CredentialCache::new(DualLocalCache {
        memory: Arc::new(Mutex::new(memory)),
        default_in_memory_ttl: None,
        diagnostics,
    })
    .map_err(failure)?;
    Ok(Arc::new(cache))
}

pub(super) fn client() -> Result<ProxiedClient, Box<HelperError>> {
    let environment = BTreeMap::new();
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment).map_err(failure)?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(failure)
}
