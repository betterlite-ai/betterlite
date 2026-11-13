use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_context::local_ttl_cache::dual::{CacheEvent, Diagnostics, DualLocalCache};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_redact::secret::Redactor;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

use crate::aws_settings::credential_cache::CredentialCache;
use crate::bedrock_settings::{BedrockError, ResponseBody};
use crate::s3_access::managed::ManagedObjectError;
use crate::s3_access::settings::{SettingMap, SettingValue};
use crate::s3_access::{
    S3AccessError, S3AccessParams, TrustedModelCredentials, TrustedValue, frozen_mapping,
};

pub(super) struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no network in tests".to_owned(),
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

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn snapshot(pairs: &[(&str, TrustedValue)]) -> TrustedModelCredentials {
    frozen_mapping(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone())),
    )
}

pub(super) fn params(trusted: TrustedModelCredentials) -> S3AccessParams {
    S3AccessParams {
        trusted,
        bucket_name: None,
        output_bucket_name: None,
    }
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

pub(super) fn credential_cache() -> Arc<CredentialCache> {
    let memory = LocalTtlCache::with_clock(Some(1000), None, None, 1024, Arc::new(|| 0.0_f64))
        .expect("the fixture cache builds");
    let emit = Arc::new(|_event: CacheEvent| {});
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(16).expect("the fixture redactor builds")),
        emit,
    };
    Arc::new(
        CredentialCache::new(DualLocalCache {
            memory: Arc::new(Mutex::new(memory)),
            default_in_memory_ttl: None,
            diagnostics,
        })
        .expect("the credential cache takes capacity 1000"),
    )
}

pub(super) fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &environment(&[])).expect("the proxy policy"),
    );
    ProxiedClient::outbound(&policy).expect("the pooled client builds")
}

pub(super) fn settings(pairs: &[(&str, &str)]) -> SettingMap {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SettingValue::Text((*value).to_owned())))
        .collect()
}

pub(super) fn store(error: &S3AccessError) -> &'static str {
    match error {
        S3AccessError::BucketRequired => "bucket_required",
        S3AccessError::BucketNameType => "bucket_name_type",
        S3AccessError::OutputBucketNameType => "output_bucket_name_type",
        S3AccessError::ManagedFileId => "managed_file_id",
        S3AccessError::InvalidS3Uri { .. } => "invalid_s3_uri",
        S3AccessError::Storage(_) => "storage",
        S3AccessError::Secret(_) => "secret",
    }
}

pub(super) fn rendered(error: &ManagedObjectError) -> String {
    match error {
        ManagedObjectError::Configuration(source) => format!("{source} {source:?}"),
        ManagedObjectError::Rejected(source) => format!("{} {source:?}", source.error.message),
    }
}

pub(super) fn assert_bedrock_rejection(error: &BedrockError, message: &str) {
    assert_eq!(error.error.status_code, 400);
    assert!(!error.error.status_code_is_synthesized);
    assert_eq!(error.error.message, message);
    assert_eq!(error.error.headers, None);
    assert_eq!(
        error.error.request,
        ErrorRequest {
            method: "POST".to_owned(),
            url: "https://docs.litellm.ai/docs".to_owned(),
        }
    );
    assert_eq!(
        error.error.response,
        ErrorResponse {
            status_code: 400,
            request: ErrorRequest {
                method: "GET".to_owned(),
                url: "https://litellm.ai".to_owned(),
            },
        }
    );
    assert_eq!(error.response_body, ResponseBody::Read(message.to_owned()));
    assert!(error.response_headers.is_empty());
}
