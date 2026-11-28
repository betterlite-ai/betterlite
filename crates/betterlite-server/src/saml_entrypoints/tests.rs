#![cfg(test)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderName, Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::saml_provider::cache::SamlCache;
use betterlite_auth::saml_provider::error::{
    NO_USABLE_IDP_DETAIL, SamlProviderError, UNAVAILABLE_DETAIL, UNCONFIGURED_DETAIL,
};
use betterlite_auth::saml_provider::fetch::{MetadataFetchError, MetadataFetcher};
use betterlite_auth::saml_provider::idp_metadata::IdpSettings;
use betterlite_auth::saml_provider::provider::SamlProvider;
use betterlite_auth::saml_provider::sp_metadata::SpMetadataErrors;
use betterlite_auth::saml_provider::verify::UnavailableSamlVerifier;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::saml_provider::{SamlProviderSettings, load_saml_provider_settings};
use betterlite_config::types::ProxyTrust;
use flate2::read::DeflateDecoder;
use percent_encoding::percent_decode_str;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt as _;

use super::error::provider_error_response;
use super::fetch::ProxiedMetadataFetcher;
use super::routes::routes;
use super::state::SamlEntrypointsState;
use crate::custom_url::CustomUrls;
use crate::middleware::request_root_paths::RequestRootPath;

const NOW: i64 = 1_796_000_000;
const INTERNAL_BODY: &str =
    r#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#;

const IDP_XML: &str = r#"<?xml version="1.0"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="https://idp.example/metadata">
  <md:IDPSSODescriptor WantAuthnRequestsSigned="false" protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:KeyDescriptor use="signing">
      <ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
        <ds:X509Data><ds:X509Certificate>MIICERT</ds:X509Certificate></ds:X509Data>
      </ds:KeyInfo>
    </md:KeyDescriptor>
    <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example/sso" />
  </md:IDPSSODescriptor>
</md:EntityDescriptor>"#;

const IDP_XML_WITHOUT_CERT: &str = r#"<?xml version="1.0"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="https://idp.example/metadata">
  <md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example/sso" />
  </md:IDPSSODescriptor>
</md:EntityDescriptor>"#;

const ENTITY_ONLY_XML: &str = r#"<?xml version="1.0"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="https://idp.example/metadata" />"#;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct FakeCache {
    authn_requests: Mutex<Vec<(String, f64)>>,
    idp_writes: Mutex<Vec<(String, f64)>>,
}

#[async_trait]
impl SamlCache for FakeCache {
    async fn get_idp_settings(
        &self,
        _cache_key: &str,
    ) -> Result<Option<IdpSettings>, AuthCacheError> {
        Ok(None)
    }

    async fn put_idp_settings(
        &self,
        cache_key: &str,
        _settings: &IdpSettings,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        lock(&self.idp_writes).push((cache_key.to_owned(), ttl_seconds));
        Ok(())
    }

    async fn put_authn_request(
        &self,
        cache_key: &str,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        let request = (cache_key.to_owned(), ttl_seconds);
        lock(&self.authn_requests).push(request);
        Ok(())
    }
}

#[derive(Default)]
struct FakeFetcher {
    body: Option<Vec<u8>>,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl MetadataFetcher for FakeFetcher {
    async fn fetch(
        &self,
        url: &str,
        verify_certificate: bool,
    ) -> Result<Vec<u8>, MetadataFetchError> {
        lock(&self.calls).push(format!("{url} {verify_certificate}"));
        self.body
            .clone()
            .ok_or(MetadataFetchError::Status { status: 502 })
    }
}

fn load_settings(pairs: &[(&str, &str)]) -> SamlProviderSettings {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    load_saml_provider_settings(&environment).unwrap()
}

fn environment_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn provider(
    pairs: &[(&str, &str)],
    cache: Arc<FakeCache>,
    fetcher: Arc<FakeFetcher>,
) -> SamlProvider {
    SamlProvider {
        settings: load_settings(pairs),
        cache,
        fetcher,
        verifier: Arc::new(UnavailableSamlVerifier),
        clock: Arc::new(|| NOW),
        backend_available: true,
    }
}

fn state(provider: SamlProvider, environment: &[(&str, &str)]) -> SamlEntrypointsState {
    SamlEntrypointsState {
        provider: Arc::new(provider),
        custom_urls: CustomUrls::default(),
        trusted_network: TrustedNetworkSettings::default(),
        environment: environment_map(environment),
    }
}

fn configured_state(xml: &str, pairs: &[(&str, &str)]) -> SamlEntrypointsState {
    let cache = Arc::new(FakeCache::default());
    let fetcher = Arc::new(FakeFetcher::default());
    state(
        provider(
            &[
                ("SAML_IDP_METADATA_XML", xml),
                ("SAML_WANT_ASSERTIONS_SIGNED", "true"),
            ],
            cache,
            fetcher,
        ),
        pairs,
    )
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::HOST, "proxy.test")
        .body(Body::empty())
        .unwrap()
}

async fn call(router: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body.to_vec())
}

fn header_of(headers: &HeaderMap, name: HeaderName) -> String {
    headers.get(name).unwrap().to_str().unwrap().to_owned()
}

fn request_id_of(cookie: &str) -> String {
    cookie
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("litellm_saml_authn=")
        .unwrap()
        .to_owned()
}

fn relay_state_of(location: &str) -> String {
    let rest = location.split_once("RelayState=").unwrap().1;
    rest.to_owned()
}

fn authn_request(location: &str) -> String {
    let query = location.split_once('?').unwrap().1;
    let encoded = query
        .split('&')
        .next()
        .unwrap()
        .strip_prefix("SAMLRequest=")
        .unwrap();
    let decoded = percent_decode_str(encoded).decode_utf8().unwrap();
    let deflated = STANDARD.decode(decoded.as_bytes()).unwrap();
    let mut xml = String::new();
    DeflateDecoder::new(deflated.as_slice())
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

fn text_of(body: &[u8]) -> String {
    String::from_utf8(body.to_vec()).unwrap()
}

fn detail_body(detail: &str) -> String {
    format!(r#"{{"detail":"{detail}"}}"#)
}

#[tokio::test]
async fn login_redirects_to_the_idp_with_the_state_cookie() {
    let cache = Arc::new(FakeCache::default());
    let fetcher = Arc::new(FakeFetcher::default());
    let provider = provider(
        &[("SAML_IDP_METADATA_XML", IDP_XML)],
        Arc::clone(&cache),
        fetcher,
    );
    let router = routes(state(provider, &[]));
    let request = get("/sso/saml/login?return_to=https%3A%2F%2Fapp.example%2Fafter");
    let (status, headers, _body) = call(router, request).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = header_of(&headers, header::LOCATION);
    assert!(
        location.starts_with("https://idp.example/sso?SAMLRequest="),
        "{location}"
    );
    assert!(
        location.ends_with("&RelayState=https%3A%2F%2Fapp.example%2Fafter"),
        "{location}"
    );
    let xml = authn_request(&location);
    for needle in [
        r#"AssertionConsumerServiceURL="http://proxy.test/sso/saml/callback""#,
        r#"Destination="https://idp.example/sso""#,
        "<saml:Issuer>http://proxy.test/sso/saml/metadata</saml:Issuer>",
    ] {
        assert!(xml.contains(needle), "{xml}");
    }
    let cookie = header_of(&headers, header::SET_COOKIE);
    let request_id = request_id_of(&cookie);
    assert_eq!(
        cookie,
        format!("litellm_saml_authn={request_id}; HttpOnly; Max-Age=600; Path=/; SameSite=lax")
    );
    assert_eq!(
        lock(&cache.authn_requests).clone(),
        vec![(format!("saml_authn_request:{request_id}"), 600.0_f64)]
    );
}

#[tokio::test]
async fn login_relay_state_defaults_to_the_callback_url() {
    let router = routes(configured_state(IDP_XML, &[]));
    let (status, headers, _body) = call(router, get("/sso/saml/login")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = header_of(&headers, header::LOCATION);
    assert_eq!(
        relay_state_of(&location),
        "http%3A%2F%2Fproxy.test%2Fsso%2Fsaml%2Fcallback"
    );
}

#[tokio::test]
async fn login_keeps_the_last_repeated_return_to() {
    let router = routes(configured_state(IDP_XML, &[]));
    let (status, headers, _body) = call(
        router,
        get("/sso/saml/login?return_to=first&return_to=second"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        relay_state_of(&header_of(&headers, header::LOCATION)),
        "second"
    );
}

#[tokio::test]
async fn login_builds_the_urls_from_the_public_base_url() {
    let mut state = configured_state(IDP_XML, &[]);
    state.custom_urls = CustomUrls {
        proxy_base_url: Some("https://public.example/base".to_owned()),
    };
    let router = routes(state);
    let (status, headers, _body) = call(router, get("/sso/saml/login")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = header_of(&headers, header::LOCATION);
    assert_eq!(
        relay_state_of(&location),
        "https%3A%2F%2Fpublic.example%2Fsso%2Fsaml%2Fcallback"
    );
    let xml = authn_request(&location);
    assert!(
        xml.contains(
            r#"AssertionConsumerServiceURL="https://public.example/base/sso/saml/callback""#
        ),
        "{xml}"
    );
}

#[tokio::test]
async fn login_with_a_secure_request_sets_the_none_same_site_cookie() {
    let router = routes(configured_state(IDP_XML, &[]));
    let request = Request::builder()
        .uri("https://proxy.test/sso/saml/login")
        .body(Body::empty())
        .unwrap();
    let (status, headers, _body) = call(router, request).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = header_of(&headers, header::SET_COOKIE);
    let request_id = request_id_of(&cookie);
    assert_eq!(
        cookie,
        format!(
            "litellm_saml_authn={request_id}; HttpOnly; Max-Age=600; Path=/; SameSite=none; Secure"
        )
    );
}

#[tokio::test]
async fn metadata_returns_the_service_provider_document() {
    let router = routes(configured_state(IDP_XML, &[]));
    let (status, headers, body) = call(router, get("/sso/saml/metadata")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/xml"
    );
    let metadata = text_of(&body);
    assert!(
        metadata.starts_with(
            "<?xml version=\"1.0\"?>\n<md:EntityDescriptor xmlns:md=\"urn:oasis:names:tc:SAML:2.0:metadata\""
        ),
        "{metadata}"
    );
    assert!(
        metadata.contains(r#"entityID="http://proxy.test/sso/saml/metadata""#),
        "{metadata}"
    );
    assert!(
        metadata.contains(r#"Location="http://proxy.test/sso/saml/callback""#),
        "{metadata}"
    );
    assert!(
        metadata.contains(r#"AuthnRequestsSigned="false""#),
        "{metadata}"
    );
    assert!(
        metadata.contains(r#"WantAssertionsSigned="true""#),
        "{metadata}"
    );
    assert!(metadata.contains(r#"validUntil=""#), "{metadata}");
}

#[tokio::test]
async fn metadata_uses_the_public_base_url_and_the_root_path() {
    let mut state = configured_state(IDP_XML, &[]);
    state.custom_urls = CustomUrls {
        proxy_base_url: Some("https://public.example/base".to_owned()),
    };
    let router = routes(state);
    let (status, _headers, body) = call(router, get("/sso/saml/metadata")).await;
    assert_eq!(status, StatusCode::OK);
    let metadata = text_of(&body);
    assert!(
        metadata.contains(r#"entityID="https://public.example/base/sso/saml/metadata""#),
        "{metadata}"
    );
    assert!(
        metadata.contains(r#"Location="https://public.example/base/sso/saml/callback""#),
        "{metadata}"
    );

    let second_router = routes(configured_state(IDP_XML, &[]));
    let request = Request::builder()
        .uri("/sso/saml/metadata")
        .header(header::HOST, "proxy.test")
        .extension(RequestRootPath("/tenant".to_owned()))
        .body(Body::empty())
        .unwrap();
    let (second_status, _, second_body) = call(second_router, request).await;
    assert_eq!(second_status, StatusCode::OK);
    let second_metadata = text_of(&second_body);
    assert!(
        second_metadata.contains(r#"entityID="http://proxy.test/tenant/sso/saml/metadata""#),
        "{second_metadata}"
    );
}

#[rstest]
#[case::true_value("true", true)]
#[case::padded_true(" TRUE ", true)]
#[case::one("1", false)]
#[case::yes("yes", false)]
#[case::on("on", false)]
#[case::empty("", false)]
#[case::false_value("false", false)]
#[tokio::test]
async fn login_reads_disable_admin_ui_as_str_to_bool(#[case] value: &str, #[case] disabled: bool) {
    let router = routes(configured_state(IDP_XML, &[("DISABLE_ADMIN_UI", value)]));
    let (status, _headers, body) = call(router, get("/sso/saml/login")).await;
    if disabled {
        assert_eq!(status, StatusCode::OK);
        assert!(text_of(&body).contains("Admin UI is Disabled"));
    } else {
        assert_eq!(status, StatusCode::SEE_OTHER);
    }
}

#[tokio::test]
async fn metadata_stays_available_when_the_admin_ui_is_disabled() {
    let router = routes(configured_state(IDP_XML, &[("DISABLE_ADMIN_UI", "true")]));
    let (status, headers, _body) = call(router, get("/sso/saml/metadata")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/xml"
    );
}

#[tokio::test]
async fn a_provider_without_configuration_answers_501_on_both_routes() {
    let router = routes(configured_state_empty());
    let (status, _headers, body) = call(router, get("/sso/saml/login")).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(text_of(&body), detail_body(UNCONFIGURED_DETAIL));
    let second_router = routes(configured_state_empty());
    let (second_status, _, second_body) = call(second_router, get("/sso/saml/metadata")).await;
    assert_eq!(second_status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(text_of(&second_body), detail_body(UNCONFIGURED_DETAIL));
}

fn configured_state_empty() -> SamlEntrypointsState {
    let cache = Arc::new(FakeCache::default());
    let fetcher = Arc::new(FakeFetcher::default());
    state(provider(&[], cache, fetcher), &[])
}

#[tokio::test]
async fn a_missing_backend_answers_501_on_both_routes() {
    let mut provider = provider(
        &[("SAML_IDP_METADATA_XML", IDP_XML)],
        Arc::new(FakeCache::default()),
        Arc::new(FakeFetcher::default()),
    );
    provider.backend_available = false;
    let router = routes(state(provider, &[]));
    let (status, _headers, body) = call(router, get("/sso/saml/login")).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(text_of(&body), detail_body(UNAVAILABLE_DETAIL));
}

#[tokio::test]
async fn metadata_without_a_usable_idp_answers_502() {
    let router = routes(configured_state(ENTITY_ONLY_XML, &[]));
    let (status, _headers, body) = call(router, get("/sso/saml/metadata")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(text_of(&body), detail_body(NO_USABLE_IDP_DETAIL));
}

#[tokio::test]
async fn invalid_sp_metadata_answers_the_metadata_detail() {
    let response = provider_error_response(&SamlProviderError::InvalidMetadata(SpMetadataErrors {
        codes: vec!["expired_xml"],
    }));
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        text_of(&body),
        detail_body("Invalid SP metadata: expired_xml")
    );
}

#[tokio::test]
async fn login_with_invalid_toolkit_settings_answers_the_configuration_detail() {
    let router = routes(configured_state(IDP_XML_WITHOUT_CERT, &[]));
    let (status, _headers, body) = call(router, get("/sso/saml/login")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        text_of(&body),
        detail_body(
            "Invalid SAML configuration: Invalid dict settings: idp_cert_or_fingerprint_not_found_and_required"
        )
    );
}

#[tokio::test]
async fn metadata_checks_only_the_sp_settings() {
    let router = routes(configured_state(IDP_XML_WITHOUT_CERT, &[]));
    let (status, _headers, _body) = call(router, get("/sso/saml/metadata")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn metadata_parse_and_fetch_failures_answer_the_unhandled_500() {
    let router = routes(configured_state("<not-xml", &[]));
    let (status, _headers, body) = call(router, get("/sso/saml/metadata")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text_of(&body), INTERNAL_BODY);

    let cache = Arc::new(FakeCache::default());
    let fetcher = Arc::new(FakeFetcher::default());
    let provider = provider(
        &[("SAML_IDP_METADATA_URL", "https://idp.example/metadata.xml")],
        cache,
        Arc::clone(&fetcher),
    );
    let second_router = routes(state(provider, &[]));
    let (second_status, _, second_body) = call(second_router, get("/sso/saml/login")).await;
    assert_eq!(second_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text_of(&second_body), INTERNAL_BODY);
    assert_eq!(
        lock(&fetcher.calls).clone(),
        vec![String::from("https://idp.example/metadata.xml true")]
    );
}

#[tokio::test]
async fn the_fetcher_follows_redirects_and_reads_only_a_200_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fetcher = ProxiedMetadataFetcher::new(&policy()).unwrap();
    let serving = async {
        let first = answer(&listener, "HTTP/1.1 302 Found\r\nlocation: /final\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
        let second = answer(
            &listener,
            "HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello",
        )
        .await;
        (first, second)
    };
    let start = format!("{base}/start");
    let (outcome, (first, second)) = tokio::join!(fetcher.fetch(&start, true), serving);
    assert_eq!(outcome.unwrap(), b"hello");
    assert!(first.starts_with("GET /start "), "{first}");
    assert!(second.starts_with("GET /final "), "{second}");
}

#[tokio::test]
async fn the_fetcher_refuses_a_status_other_than_200() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fetcher = ProxiedMetadataFetcher::new(&policy()).unwrap();
    let serving = answer(
        &listener,
        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    );
    let missing = format!("{base}/missing");
    let (outcome, _request) = tokio::join!(fetcher.fetch(&missing, false), serving);
    assert!(matches!(
        outcome,
        Err(MetadataFetchError::Status { status: 404 })
    ));
}

fn policy() -> Arc<ProxyPolicy> {
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap())
}

async fn answer(listener: &TcpListener, response: &str) -> String {
    let (mut stream, _peer) = listener.accept().await.unwrap();
    let request = read_request(&mut stream).await;
    stream.write_all(response.as_bytes()).await.unwrap();
    request
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    String::from_utf8(received).unwrap()
}
