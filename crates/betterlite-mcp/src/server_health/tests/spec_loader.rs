use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};

use crate::openapi_tools::error::OpenApiError;
use crate::server_health::openapi::MAX_SPEC_BYTES;
use crate::server_health::ports::SpecLoader as _;
use crate::server_health::spec_loader::OpenApiSpecLoader;

// the real loader must pass a failed read through instead of reporting a loaded spec
#[tokio::test]
async fn a_missing_spec_file_is_reported() {
    let trust = ProxyTrust::default();
    let policy = Arc::new(ProxyPolicy::resolve(&trust, &BTreeMap::new()).unwrap());
    let client =
        Arc::new(ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap());
    let loader = OpenApiSpecLoader {
        fetcher: UserUrlFetcher {
            validator: UrlValidator::new(SystemResolver, &[], &Verification::Enabled),
            validation: false,
        },
        client,
    };
    let missing = format!("{}/no-such-spec.json", env!("CARGO_MANIFEST_DIR"));
    let error = loader.load(&missing, MAX_SPEC_BYTES).await.unwrap_err();
    assert!(matches!(error, OpenApiError::SpecNotFound { .. }));
}
