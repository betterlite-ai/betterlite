#![cfg(test)]

mod hardening;

use std::io::Read as _;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::backend::{CacheResult, Ttl};
use betterlite_cache::dual_cache::{CacheOptions, DualCacheError, MemoryMaxSize};
use betterlite_config::saml_provider::{SamlFlag, SamlProviderSettings};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use flate2::read::DeflateDecoder;
use rstest::rstest;
use sha2::{Digest as _, Sha256};

use crate::auth_cache::backend::{AuthCacheTier, PreCall};
use crate::auth_cache::{AuthCache, AuthCacheError};

use super::authn_request::{authn_request_xml, generate_request_id, redirect_location};
use super::cache::{AUTHN_TTL, IDP_TTL, SamlCache};
use super::error::SamlProviderError;
use super::fetch::{MetadataFetchError, MetadataFetcher};
use super::idp_metadata::{IdpSettings, X509CertMulti, decode_saml_metadata};
use super::provider::{ProviderUrls, SamlProvider};
use super::settings::{MergedSettings, validate_url};
use super::sp_metadata::validate_sp_metadata;
use super::verify::{
    SamlResponseVerifier, SamlVerificationError, UnavailableSamlVerifier, VerifiedResponse,
    VerifyRequest,
};

const NOW: i64 = 1_796_000_000;
const ACS_URL: &str = "https://proxy.example/sso/saml/callback";
const METADATA_URL: &str = "https://proxy.example/sso/saml/metadata";
const IDP_SSO_URL: &str = "https://idp.example.com/sso";
const RELAY: &str = "https://app.example/after";

const IDP_XML: &str = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" entityID="urn:example:idp"><md:IDPSSODescriptor><md:NameIDFormat>urn:oasis:names:tc:SAML:2.0:nameid-format:persistent</md:NameIDFormat><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example.com/sso"/><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>CERT</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;

const IDP_XML_MINIMAL: &str = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" entityID="urn:example:idp"><md:IDPSSODescriptor><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example.com/sso"/><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>CERT</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;

const ENTITY_ONLY_XML: &str = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="urn:example:idp"/>"#;

fn provider_settings(url: Option<&str>, xml: Option<&str>) -> SamlProviderSettings {
    SamlProviderSettings {
        idp_metadata_url: url.map(str::to_owned),
        idp_metadata_xml: xml.map(str::to_owned),
        sp_entity_id: None,
        sp_name_id_format: None,
        strict: SamlFlag(true),
        want_assertions_signed: SamlFlag(true),
        want_messages_signed: SamlFlag(false),
        authn_requests_signed: SamlFlag(false),
        idp_metadata_validate_cert: SamlFlag(true),
    }
}

fn urls() -> ProviderUrls {
    ProviderUrls {
        metadata_url: METADATA_URL.to_owned(),
        acs_url: ACS_URL.to_owned(),
    }
}

fn cache_key(prefix: &str, source: &str) -> String {
    let digest = Sha256::digest(source.as_bytes());
    format!("{prefix}:{}", hex::encode(digest))
}

#[derive(Default)]
struct CacheState {
    idp: Option<IdpSettings>,
    reads: Vec<String>,
    idp_writes: Vec<(String, IdpSettings, f64)>,
    authn_writes: Vec<(String, f64)>,
}

#[derive(Clone, Default)]
struct FakeCache {
    state: Arc<Mutex<CacheState>>,
}

impl FakeCache {
    fn seeded(idp: IdpSettings) -> Self {
        let cache = Self::default();
        cache.state().idp = Some(idp);
        cache
    }

    fn state(&self) -> MutexGuard<'_, CacheState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl SamlCache for FakeCache {
    async fn get_idp_settings(
        &self,
        cache_key: &str,
    ) -> Result<Option<IdpSettings>, AuthCacheError> {
        let mut state = self.state();
        state.reads.push(cache_key.to_owned());
        Ok(state.idp.clone())
    }

    async fn put_idp_settings(
        &self,
        cache_key: &str,
        settings: &IdpSettings,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        self.state()
            .idp_writes
            .push((cache_key.to_owned(), settings.clone(), ttl_seconds));
        Ok(())
    }

    async fn put_authn_request(
        &self,
        cache_key: &str,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        self.state()
            .authn_writes
            .push((cache_key.to_owned(), ttl_seconds));
        Ok(())
    }
}

#[derive(Default)]
struct FetchState {
    calls: Vec<(String, bool)>,
    body: Vec<u8>,
    fail: bool,
}

#[derive(Clone, Default)]
struct FakeFetcher {
    state: Arc<Mutex<FetchState>>,
}

impl FakeFetcher {
    fn returning(body: &str) -> Self {
        let fetcher = Self::default();
        fetcher.state().body = body.as_bytes().to_vec();
        fetcher
    }

    fn failing() -> Self {
        let fetcher = Self::default();
        fetcher.state().fail = true;
        fetcher
    }

    fn calls(&self) -> Vec<(String, bool)> {
        self.fetcher_state().calls.clone()
    }

    fn fetcher_state(&self) -> MutexGuard<'_, FetchState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl MetadataFetcher for FakeFetcher {
    async fn fetch(
        &self,
        url: &str,
        verify_certificate: bool,
    ) -> Result<Vec<u8>, MetadataFetchError> {
        let mut state = self.fetcher_state();
        state.calls.push((url.to_owned(), verify_certificate));
        if state.fail {
            return Err(MetadataFetchError::Status { status: 500 });
        }
        Ok(state.body.clone())
    }
}

struct RejectingVerifier;

#[async_trait]
impl SamlResponseVerifier for RejectingVerifier {
    async fn verify(
        &self,
        request: &VerifyRequest<'_>,
    ) -> Result<VerifiedResponse, SamlVerificationError> {
        Err(SamlVerificationError::Rejected {
            reason: request.expected_request_id.unwrap_or_default().to_owned(),
        })
    }
}

fn build_provider(
    settings: SamlProviderSettings,
    cache: &FakeCache,
    fetcher: &FakeFetcher,
) -> SamlProvider {
    SamlProvider {
        settings,
        cache: Arc::new(cache.clone()),
        fetcher: Arc::new(fetcher.clone()),
        verifier: Arc::new(UnavailableSamlVerifier),
        clock: Arc::new(|| NOW),
        backend_available: true,
    }
}

fn inline_provider(xml: &str) -> (SamlProvider, FakeCache, FakeFetcher) {
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::default();
    let provider = build_provider(provider_settings(None, Some(xml)), &cache, &fetcher);
    (provider, cache, fetcher)
}

fn query_value(location: &str, name: &str) -> String {
    let marker = format!("{name}=");
    let (_prefix, tail) = location.split_once(&marker).unwrap();
    let value = tail.split_once('&').map_or(tail, |(value, _rest)| value);
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .unwrap()
        .into_owned()
}

fn inflated_request(location: &str) -> String {
    let encoded = query_value(location, "SAMLRequest");
    let compressed = STANDARD.decode(encoded).unwrap();
    let mut xml = String::new();
    DeflateDecoder::new(compressed.as_slice())
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

#[tokio::test]
async fn login_redirects_with_the_toolkit_authn_request() {
    let (provider, cache, fetcher) = inline_provider(IDP_XML);
    let redirect = provider.login(&urls(), RELAY).await.unwrap();
    assert!(
        redirect
            .location
            .starts_with("https://idp.example.com/sso?SAMLRequest=")
    );
    assert!(
        redirect
            .location
            .contains("&RelayState=https%3A%2F%2Fapp.example%2Fafter")
    );
    let expected = format!(
        "<samlp:AuthnRequest\n  xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\"\n  xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\"\n  ID=\"{0}\"\n  Version=\"2.0\"\n  IssueInstant=\"2026-11-30T00:53:20Z\"\n  Destination=\"https://idp.example.com/sso\"\n  ProtocolBinding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\"\n  AssertionConsumerServiceURL=\"https://proxy.example/sso/saml/callback\">\n    <saml:Issuer>https://proxy.example/sso/saml/metadata</saml:Issuer>\n    <samlp:NameIDPolicy\n        Format=\"urn:oasis:names:tc:SAML:2.0:nameid-format:persistent\"\n        AllowCreate=\"true\" />\n\n</samlp:AuthnRequest>",
        redirect.request_id
    );
    assert_eq!(inflated_request(&redirect.location), expected);
    assert!(fetcher.calls().is_empty());
    let state = cache.state();
    assert_eq!(
        state.authn_writes,
        vec![(
            format!("saml_authn_request:{}", redirect.request_id),
            AUTHN_TTL
        )]
    );
    assert_eq!(
        state.idp_writes,
        vec![(
            cache_key("saml_idp_settings", IDP_XML),
            IdpSettings {
                entity_id: Some("urn:example:idp".to_owned()),
                sso_url: Some(IDP_SSO_URL.to_owned()),
                slo_url: None,
                x509_cert: Some("CERT".to_owned()),
                x509_cert_multi: None,
                name_id_format: Some(
                    "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent".to_owned()
                ),
                want_authn_requests_signed: None,
            },
            IDP_TTL
        )]
    );
}

#[tokio::test]
async fn login_quotes_the_relay_state_like_quote_plus() {
    let (provider, _cache, _fetcher) = inline_provider(IDP_XML);
    let redirect = provider.login(&urls(), "a b&c/d?e").await.unwrap();
    assert!(redirect.location.ends_with("&RelayState=a+b%26c%2Fd%3Fe"));
}

#[tokio::test]
async fn login_uses_a_configured_sp_entity_id_and_name_id_format() {
    let (provider, _cache, _fetcher) = inline_provider(IDP_XML_MINIMAL);
    let mut settings = provider.settings.clone();
    settings.sp_entity_id = Some("urn:proxy:sp".to_owned());
    settings.sp_name_id_format =
        Some("urn:oasis:names:tc:SAML:2.0:nameid-format:transient".to_owned());
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::default();
    let configured = build_provider(settings, &cache, &fetcher);
    let redirect = configured.login(&urls(), RELAY).await.unwrap();
    let xml = inflated_request(&redirect.location);
    assert!(xml.contains("<saml:Issuer>urn:proxy:sp</saml:Issuer>"));
    assert!(xml.contains("Format=\"urn:oasis:names:tc:SAML:2.0:nameid-format:transient\""));
}

#[tokio::test]
async fn login_takes_the_name_id_format_from_the_metadata_first() {
    let (provider, _cache, _fetcher) = inline_provider(IDP_XML);
    let mut settings = provider.settings.clone();
    settings.sp_name_id_format =
        Some("urn:oasis:names:tc:SAML:2.0:nameid-format:transient".to_owned());
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::default();
    let configured = build_provider(settings, &cache, &fetcher);
    let redirect = configured.login(&urls(), RELAY).await.unwrap();
    assert!(
        inflated_request(&redirect.location)
            .contains("Format=\"urn:oasis:names:tc:SAML:2.0:nameid-format:persistent\"")
    );
}

#[tokio::test]
async fn a_cached_idp_settings_skips_the_fetcher() {
    let cached = IdpSettings {
        entity_id: Some("urn:example:idp".to_owned()),
        sso_url: Some(IDP_SSO_URL.to_owned()),
        slo_url: None,
        x509_cert: Some("CERT".to_owned()),
        x509_cert_multi: None,
        name_id_format: None,
        want_authn_requests_signed: None,
    };
    let cache = FakeCache::seeded(cached);
    let fetcher = FakeFetcher::failing();
    let provider = build_provider(
        provider_settings(Some("https://idp.example.com/metadata"), None),
        &cache,
        &fetcher,
    );
    provider.login(&urls(), RELAY).await.unwrap();
    assert!(fetcher.calls().is_empty());
    assert_eq!(
        cache.state().reads,
        vec![cache_key(
            "saml_idp_settings",
            "https://idp.example.com/metadata"
        )]
    );
}

#[tokio::test]
async fn remote_metadata_is_fetched_with_the_configured_certificate_check() {
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::returning(IDP_XML);
    let mut settings = provider_settings(Some("https://idp.example.com/metadata"), None);
    settings.idp_metadata_validate_cert = SamlFlag(false);
    let provider = build_provider(settings, &cache, &fetcher);
    provider.login(&urls(), RELAY).await.unwrap();
    assert_eq!(
        fetcher.calls(),
        vec![("https://idp.example.com/metadata".to_owned(), false)]
    );
    let state = cache.state();
    assert_eq!(
        state.idp_writes[0].0,
        cache_key("saml_idp_settings", "https://idp.example.com/metadata")
    );
}

#[tokio::test]
async fn an_empty_metadata_url_fetches_remotely_while_the_xml_supplies_the_cache_key() {
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::returning(IDP_XML);
    let provider = build_provider(provider_settings(Some(""), Some(IDP_XML)), &cache, &fetcher);
    provider.login(&urls(), RELAY).await.unwrap();
    assert_eq!(fetcher.calls(), vec![(String::new(), true)]);
    assert_eq!(
        cache.state().idp_writes[0].0,
        cache_key("saml_idp_settings", IDP_XML)
    );
}

#[tokio::test]
async fn a_configured_url_wins_the_cache_source_over_the_xml() {
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::returning(IDP_XML);
    let provider = build_provider(
        provider_settings(
            Some("https://idp.example.com/metadata"),
            Some(IDP_XML_MINIMAL),
        ),
        &cache,
        &fetcher,
    );
    provider.login(&urls(), RELAY).await.unwrap();
    assert_eq!(
        cache.state().idp_writes[0].0,
        cache_key("saml_idp_settings", "https://idp.example.com/metadata")
    );
}

#[tokio::test]
async fn an_unconfigured_provider_is_the_toolkit_unconfigured_error() {
    let (provider, cache, fetcher) = inline_provider("");
    let mut settings = provider.settings.clone();
    settings.idp_metadata_xml = None;
    let empty = build_provider(settings, &cache, &fetcher);
    assert!(matches!(
        empty.login(&urls(), RELAY).await,
        Err(SamlProviderError::Unconfigured)
    ));
    assert!(matches!(
        empty.sp_metadata(&urls()).await,
        Err(SamlProviderError::Unconfigured)
    ));
}

#[tokio::test]
async fn metadata_without_an_identity_provider_is_a_bad_gateway() {
    let (provider, _cache, _fetcher) = inline_provider(ENTITY_ONLY_XML);
    assert!(matches!(
        provider.login(&urls(), RELAY).await,
        Err(SamlProviderError::NoUsableIdp)
    ));
    let empty_descriptor = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor/></md:EntityDescriptor>"#;
    let (provider, _cache, _fetcher) = inline_provider(empty_descriptor);
    assert!(matches!(
        provider.login(&urls(), RELAY).await,
        Err(SamlProviderError::NoUsableIdp)
    ));
}

#[tokio::test]
async fn remote_metadata_without_an_idp_descriptor_is_a_bad_gateway() {
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::returning(ENTITY_ONLY_XML);
    let provider = build_provider(
        provider_settings(Some("https://idp.example.com/metadata"), None),
        &cache,
        &fetcher,
    );
    assert!(matches!(
        provider.sp_metadata(&urls()).await,
        Err(SamlProviderError::NoUsableIdp)
    ));
}

#[tokio::test]
async fn a_fetch_failure_stays_a_fetch_error() {
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::failing();
    let provider = build_provider(
        provider_settings(Some("https://idp.example.com/metadata"), None),
        &cache,
        &fetcher,
    );
    assert!(matches!(
        provider.login(&urls(), RELAY).await,
        Err(SamlProviderError::Fetch(_))
    ));
}

#[tokio::test]
async fn invalid_settings_carry_every_toolkit_code() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example.com/sso"/></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let (provider, _cache, _fetcher) = inline_provider(xml);
    let error = provider.login(&urls(), RELAY).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid SAML configuration: Invalid dict settings: idp_entityId_not_found,idp_cert_or_fingerprint_not_found_and_required"
    );
}

#[tokio::test]
async fn metadata_want_authn_requests_signed_fails_the_sp_certificate_check() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" entityID="urn:example:idp"><md:IDPSSODescriptor WantAuthnRequestsSigned="true"><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example.com/sso"/><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>CERT</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let (provider, _cache, _fetcher) = inline_provider(xml);
    let error = provider.login(&urls(), RELAY).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid SAML configuration: Invalid dict settings: sp_cert_not_found_and_required"
    );
}

#[tokio::test]
async fn an_ftp_single_sign_on_url_validates_and_then_cannot_be_redirected_to() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" entityID="urn:example:idp"><md:IDPSSODescriptor><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="ftp://idp.example.com/sso"/><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>CERT</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let (provider, _cache, _fetcher) = inline_provider(xml);
    assert!(matches!(
        provider.login(&urls(), RELAY).await,
        Err(SamlProviderError::InvalidRedirectUrl)
    ));
}

#[tokio::test]
async fn sp_metadata_publishes_the_toolkit_template() {
    let (provider, _cache, _fetcher) = inline_provider(IDP_XML_MINIMAL);
    let metadata = provider.sp_metadata(&urls()).await.unwrap();
    let expected = "<?xml version=\"1.0\"?>\n<md:EntityDescriptor xmlns:md=\"urn:oasis:names:tc:SAML:2.0:metadata\"\n                     validUntil=\"2026-12-02T00:53:20Z\"\n                     cacheDuration=\"PT604800S\"\n                     entityID=\"https://proxy.example/sso/saml/metadata\">\n    <md:SPSSODescriptor AuthnRequestsSigned=\"false\" WantAssertionsSigned=\"true\" protocolSupportEnumeration=\"urn:oasis:names:tc:SAML:2.0:protocol\">\n        <md:NameIDFormat>urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress</md:NameIDFormat>\n        <md:AssertionConsumerService Binding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\"\n                                     Location=\"https://proxy.example/sso/saml/callback\"\n                                     index=\"1\" />\n    </md:SPSSODescriptor>\n\n\n</md:EntityDescriptor>";
    assert_eq!(metadata, expected);
}

#[tokio::test]
async fn sp_metadata_skips_the_identity_provider_checks() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID=""><md:IDPSSODescriptor><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="not-a-url"/></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let (provider, _cache, _fetcher) = inline_provider(xml);
    let metadata = provider.sp_metadata(&urls()).await.unwrap();
    assert!(metadata.contains("WantAssertionsSigned=\"true\""));
}

#[tokio::test]
async fn sp_metadata_reports_sp_settings_as_an_internal_error() {
    let (provider, _cache, _fetcher) = inline_provider(IDP_XML);
    let mut settings = provider.settings.clone();
    settings.sp_entity_id = Some(String::new());
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::default();
    let configured = build_provider(settings, &cache, &fetcher);
    let error = configured.sp_metadata(&urls()).await.unwrap_err();
    assert!(matches!(
        error,
        SamlProviderError::InvalidMetadataSettings(_)
    ));
}

#[tokio::test]
async fn an_unavailable_backend_answers_both_routes_with_the_501_state() {
    let cache = FakeCache::default();
    let fetcher = FakeFetcher::default();
    let mut provider = build_provider(provider_settings(None, Some(IDP_XML)), &cache, &fetcher);
    provider.backend_available = false;
    assert!(matches!(
        provider.login(&urls(), RELAY).await,
        Err(SamlProviderError::Unavailable)
    ));
    assert!(matches!(
        provider.sp_metadata(&urls()).await,
        Err(SamlProviderError::Unavailable)
    ));
    assert!(fetcher.calls().is_empty());
}

#[tokio::test]
async fn verify_response_never_accepts_without_the_signature_backend() {
    let (provider, _cache, _fetcher) = inline_provider(IDP_XML);
    let request = VerifyRequest {
        response_xml: b"<samlp:Response/>",
        expected_request_id: Some("REQ1"),
    };
    assert_eq!(
        provider.verify_response(&request).await,
        Err(SamlVerificationError::Unavailable)
    );
}

#[tokio::test]
async fn verify_response_reaches_the_injected_verifier() {
    let (mut provider, _cache, _fetcher) = inline_provider(IDP_XML);
    provider.verifier = Arc::new(RejectingVerifier);
    let request = VerifyRequest {
        response_xml: b"<samlp:Response/>",
        expected_request_id: Some("REQ1"),
    };
    assert_eq!(
        provider.verify_response(&request).await,
        Err(SamlVerificationError::Rejected {
            reason: "REQ1".to_owned()
        })
    );
}

#[test]
fn the_authn_request_xml_matches_the_toolkit_template() {
    let settings = MergedSettings {
        strict: SamlFlag(true),
        sp_entity_id: METADATA_URL.to_owned(),
        acs_url: ACS_URL.to_owned(),
        name_id_format: "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress".to_owned(),
        want_assertions_signed: SamlFlag(true),
        want_messages_signed: SamlFlag(false),
        authn_requests_signed: SamlFlag(false),
        idp: IdpSettings {
            entity_id: Some("urn:example:idp".to_owned()),
            sso_url: Some(IDP_SSO_URL.to_owned()),
            slo_url: None,
            x509_cert: Some("CERT".to_owned()),
            x509_cert_multi: None,
            name_id_format: None,
            want_authn_requests_signed: None,
        },
    };
    let xml = authn_request_xml(&settings, "ONELOGIN_FIXED", 1_796_000_000).unwrap();
    let expected = "<samlp:AuthnRequest\n  xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\"\n  xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\"\n  ID=\"ONELOGIN_FIXED\"\n  Version=\"2.0\"\n  IssueInstant=\"2026-11-30T00:53:20Z\"\n  Destination=\"https://idp.example.com/sso\"\n  ProtocolBinding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\"\n  AssertionConsumerServiceURL=\"https://proxy.example/sso/saml/callback\">\n    <saml:Issuer>https://proxy.example/sso/saml/metadata</saml:Issuer>\n    <samlp:NameIDPolicy\n        Format=\"urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress\"\n        AllowCreate=\"true\" />\n\n</samlp:AuthnRequest>";
    assert_eq!(xml, expected);
}

#[test]
fn the_request_id_is_a_onelogin_sha1_token() {
    let first = generate_request_id();
    assert!(first.starts_with("ONELOGIN_"));
    assert_eq!(first.len(), 49);
    assert!(
        first["ONELOGIN_".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
    assert_ne!(first, generate_request_id());
}

#[test]
fn the_redirect_appends_the_parameters_to_an_existing_query() {
    let location = redirect_location("https://idp.example.com/sso?tenant=1", "<x/>", "r").unwrap();
    assert!(location.starts_with("https://idp.example.com/sso?tenant=1&SAMLRequest="));
    redirect_location("/relative", "<x/>", "r").unwrap_err();
}

#[test]
fn x509_certificate_text_is_normalized_to_the_metadata_certificate() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" entityID="urn:x"><md:IDPSSODescriptor><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate> AA
BB </ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.x509_cert.as_deref(), Some("AABB"));
}

#[test]
fn two_signing_certificates_keep_the_multiple_certificate_shape() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><md:IDPSSODescriptor><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>AA</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>BB</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.x509_cert, None);
    assert_eq!(
        idp.x509_cert_multi,
        Some(X509CertMulti {
            signing: Some(vec!["AA".to_owned(), "BB".to_owned()]),
            encryption: None,
        })
    );
}

#[test]
fn a_signing_and_an_encryption_certificate_that_differ_stay_separate() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><md:IDPSSODescriptor><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>AA</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor><md:KeyDescriptor use="encryption"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>BB</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(
        idp.x509_cert_multi,
        Some(X509CertMulti {
            signing: Some(vec!["AA".to_owned()]),
            encryption: Some(vec!["BB".to_owned()]),
        })
    );
}

#[test]
fn one_signing_and_one_encryption_certificate_that_match_collapse_to_one() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><md:IDPSSODescriptor><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>AA</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor><md:KeyDescriptor use="encryption"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>AA</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.x509_cert.as_deref(), Some("AA"));
    assert_eq!(idp.x509_cert_multi, None);
}

#[test]
fn the_first_entity_descriptor_and_its_first_idp_descriptor_win() {
    let xml = r#"<md:EntitiesDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:EntityDescriptor entityID="urn:first"><md:IDPSSODescriptor><md:NameIDFormat>urn:persistent</md:NameIDFormat><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://first.example/sso"/></md:IDPSSODescriptor></md:EntityDescriptor><md:EntityDescriptor entityID="urn:second"><md:IDPSSODescriptor><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://second.example/sso"/></md:IDPSSODescriptor></md:EntityDescriptor></md:EntitiesDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.entity_id.as_deref(), Some("urn:first"));
    assert_eq!(idp.sso_url.as_deref(), Some("https://first.example/sso"));
    assert_eq!(idp.name_id_format.as_deref(), Some("urn:persistent"));
}

#[test]
fn only_the_first_name_id_format_is_read() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor><md:NameIDFormat/><md:NameIDFormat>urn:second</md:NameIDFormat></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.name_id_format, None);
}

#[test]
fn a_descriptor_without_values_is_an_empty_identity_provider() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor/></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert!(idp.is_empty());
}

#[test]
fn want_authn_requests_signed_is_true_only_for_the_lowercase_literal() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor WantAuthnRequestsSigned="TRUE"/></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.want_authn_requests_signed, Some(false));
}

#[test]
fn a_document_with_a_document_type_is_refused() {
    let xml = r#"<!DOCTYPE md:EntityDescriptor [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="&xxe;"/>"#;
    decode_saml_metadata(xml.as_bytes()).unwrap_err();
}

#[test]
fn an_undeclared_namespace_prefix_is_refused() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor><unknown:Element/></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    decode_saml_metadata(xml.as_bytes()).unwrap_err();
}

#[test]
fn a_certificate_element_without_text_is_refused() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><md:IDPSSODescriptor><md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate/></ds:X509Data></ds:KeyInfo></md:KeyDescriptor></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    decode_saml_metadata(xml.as_bytes()).unwrap_err();
}

#[rstest]
#[case::fqdn("https://idp.example.com/sso", true)]
#[case::localhost_port("http://localhost:8080/sso", true)]
#[case::ipv4("http://127.0.0.1/sso", true)]
#[case::ipv6("https://[::1]:8443/sso", true)]
#[case::no_path("https://idp.example.com", true)]
#[case::root_path("https://idp.example.com/", true)]
#[case::query("https://idp.example.com?x=1", true)]
#[case::single_label("https://idp", false)]
#[case::ftp("ftp://files.example.com/meta", true)]
#[case::ftps("ftps://files.example.com/meta", true)]
#[case::file_scheme("file:///etc/passwd", false)]
#[case::two_letter_tld("https://idp.app/sso", true)]
#[case::dotted_host("https://idp.example.com.", true)]
#[case::long_tld("https://idp.example.corporate/sso", true)]
#[case::uppercase("HTTPS://IDP.EXAMPLE.COM/sso", true)]
#[case::bad_port("https://idp.example.com:notaport/sso", false)]
#[case::credentials("https://user:pass@idp.example.com/sso", false)]
#[case::space("https://idp.example.com/sso path", false)]
#[case::scheme_relative("//idp.example.com/sso", false)]
#[case::empty_query("https://idp.example.com?", false)]
#[case::slash_empty_query("https://idp.example.com/?", true)]
#[case::fragment("https://idp.example.com#frag", false)]
#[case::path_with_fragment("https://idp.example.com/sso#frag", true)]
#[case::underscore_label("https://idp_x.example.com/sso", true)]
#[case::leading_hyphen("https://-idp.example.com/sso", false)]
fn validate_url_matches_the_toolkit(#[case] url: &str, #[case] valid: bool) {
    assert_eq!(validate_url(url), valid);
}

#[rstest]
#[case::empty("", "unloaded_xml")]
#[case::not_xml("not xml", "unloaded_xml")]
#[case::foreign_root("<other/>", "noEntityDescriptor_xml")]
fn sp_metadata_validation_names_the_broken_part(#[case] metadata: &str, #[case] code: &str) {
    let errors = validate_sp_metadata(metadata, NOW);
    assert_eq!(errors.codes, vec![code]);
}

#[test]
fn sp_metadata_validation_requires_exactly_one_sp_descriptor() {
    let metadata = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:SPSSODescriptor/><md:SPSSODescriptor/></md:EntityDescriptor>"#;
    let errors = validate_sp_metadata(metadata, NOW);
    assert_eq!(errors.codes, vec!["onlySPSSODescriptor_allowed_xml"]);
}

#[test]
fn sp_metadata_validation_flags_an_expired_document() {
    let metadata = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" validUntil="2020-01-01T00:00:00Z" cacheDuration="PT604800S"><md:SPSSODescriptor/></md:EntityDescriptor>"#;
    let errors = validate_sp_metadata(metadata, NOW);
    assert_eq!(errors.codes, vec!["expired_xml"]);
    assert_eq!(errors.to_string(), "Invalid SP metadata: expired_xml");
}

#[derive(Default)]
struct Recorded {
    gets: Vec<(String, bool)>,
    sets: Vec<(String, Json, CacheOptions)>,
}

struct TestMemory;

impl MemoryCacheOps for TestMemory {
    fn default_ttl(&self) -> i64 {
        60
    }

    fn get_cache(&self, _key: &str) -> CacheResult<Option<CacheValue>> {
        Ok(None)
    }

    fn set_cache(&self, _key: &str, _value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        Ok(())
    }

    fn delete_cache(&self, _key: &str) -> CacheResult<()> {
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        Ok(keys.iter().map(|_key| None).collect())
    }

    fn increment_cache(&self, _key: &str, _value: f64, _ttl: Ttl) -> CacheResult<f64> {
        Ok(0.0_f64)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        Ok(Vec::new())
    }
}

impl MemoryMaxSize for TestMemory {
    fn set_max_size(&self, _max_size: usize) {}
}

struct TestTier {
    memory: Arc<TestMemory>,
    recorded: Arc<Mutex<Recorded>>,
    cached: Option<Json>,
}

fn record(recorded: &Mutex<Recorded>) -> MutexGuard<'_, Recorded> {
    recorded.lock().unwrap_or_else(PoisonError::into_inner)
}

#[async_trait]
impl AuthCacheTier for TestTier {
    type Memory = TestMemory;

    fn in_memory(&self) -> &Arc<TestMemory> {
        &self.memory
    }

    fn memory_ttl(&self) -> Option<f64> {
        None
    }

    fn update_cache_ttl(&self, _memory_ttl: Option<f64>, _redis_ttl: Option<f64>) {}

    fn update_in_memory_max_size(&self, _max_size: Option<usize>) {}

    fn attach_redis_cache(&self, _redis: Option<Arc<RedisCache>>, _redis_ttl: Option<f64>) {}

    fn get(&self, key: &str, local_only: bool) -> Option<Json> {
        record(&self.recorded)
            .gets
            .push((key.to_owned(), local_only));
        self.cached.clone()
    }

    async fn async_get(&self, key: &str, local_only: bool) -> Option<Json> {
        self.get(key, local_only)
    }

    fn set(&self, key: Option<&str>, value: &Json, options: CacheOptions) {
        let key = key.unwrap_or_default().to_owned();
        record(&self.recorded)
            .sets
            .push((key, value.clone(), options));
    }

    async fn async_set(&self, key: Option<&str>, value: &Json, options: CacheOptions) {
        self.set(key, value, options);
    }

    fn set_pre_call(
        &self,
        _key: &str,
        _value: &Json,
        _ttl: Option<f64>,
    ) -> Result<PreCall, DualCacheError> {
        Ok(PreCall::Direct)
    }

    fn delete(&self, _key: &str) -> Result<(), DualCacheError> {
        Ok(())
    }

    async fn async_delete(&self, _key: &str) -> Result<(), DualCacheError> {
        Ok(())
    }

    fn delete_pre_call(&self, _key: &str) -> Result<Option<BatchResult<()>>, DualCacheError> {
        Ok(None)
    }

    async fn async_delete_keys(&self, _keys: &[String]) -> Result<(), DualCacheError> {
        Ok(())
    }

    async fn async_set_pipeline(
        &self,
        _entries: &[(String, Json)],
        _options: CacheOptions,
    ) -> Result<(), DualCacheError> {
        Ok(())
    }
}

#[tokio::test]
async fn the_auth_cache_stores_settings_for_an_hour_and_a_request_id_for_ten_minutes() {
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let cache = AuthCache {
        ordinary: TestTier {
            memory: Arc::new(TestMemory),
            recorded: Arc::clone(&recorded),
            cached: None,
        },
        key_objects: TestTier {
            memory: Arc::new(TestMemory),
            recorded: Arc::clone(&recorded),
            cached: None,
        },
        management_ttl_fallback: 45.0_f64,
    };
    let idp = IdpSettings {
        entity_id: Some("urn:x".to_owned()),
        sso_url: None,
        slo_url: None,
        x509_cert: None,
        x509_cert_multi: None,
        name_id_format: None,
        want_authn_requests_signed: None,
    };
    cache
        .put_idp_settings("saml_idp_settings:key", &idp, IDP_TTL)
        .await
        .unwrap();
    cache
        .put_authn_request("saml_authn_request:ONELOGIN_1", AUTHN_TTL)
        .await
        .unwrap();
    let observed = record(&recorded);
    assert_eq!(observed.sets.len(), 2);
    let (idp_key, idp_value, idp_options) = &observed.sets[0];
    assert_eq!(idp_key, "saml_idp_settings:key");
    match idp_value {
        Json::Object(map) => assert!(map.contains_key("entity_id")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the cached settings are not a JSON object")
        }
    }
    assert_eq!(idp_options.ttl, Some(3600.0_f64));
    assert!(!idp_options.local_only);
    let (request_key, request_value, request_options) = &observed.sets[1];
    assert_eq!(request_key, "saml_authn_request:ONELOGIN_1");
    assert_eq!(request_value, &Json::Text("1".to_owned()));
    assert_eq!(request_options.ttl, Some(600.0_f64));
    assert!(!request_options.local_only);
}

#[tokio::test]
async fn the_auth_cache_reads_settings_with_the_typed_decoder() {
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let cache = AuthCache {
        ordinary: TestTier {
            memory: Arc::new(TestMemory),
            recorded: Arc::clone(&recorded),
            cached: None,
        },
        key_objects: TestTier {
            memory: Arc::new(TestMemory),
            recorded: Arc::clone(&recorded),
            cached: None,
        },
        management_ttl_fallback: 45.0_f64,
    };
    let cached = cache
        .get_idp_settings("saml_idp_settings:key")
        .await
        .unwrap();
    assert_eq!(cached, None);
    assert_eq!(
        record(&recorded).gets,
        vec![("saml_idp_settings:key".to_owned(), false)]
    );
}

#[rstest]
fn replays_the_saml_metadata_corpus(
    #[files("fuzz/corpus/saml_metadata/*")] path: std::path::PathBuf,
) {
    let bytes = std::fs::read(path).unwrap();
    drop(decode_saml_metadata(&bytes));
}
