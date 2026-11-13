use std::error::Error as _;
use std::mem::discriminant;
use std::sync::Arc;

use betterlite_config::sections::azure_client_secret::Settings;
use betterlite_openai::error::{ErrorRequest, ErrorResponse, ProviderError, ProviderErrorParts};
use secrecy::{ExposeSecret as _, SecretString};

use super::fixture::{
    CLIENT, Fixture, FixtureResolver, FixtureTransport, HOST, NOW, Reply, SECRET, TENANT,
    error_text, form_map, imds, input, input_for, instance_metadata, openid, openid_for, token,
};
use crate::azure_client_secret::authority::{self, Authority};
use crate::azure_client_secret::provider::{ClientSecretProvider, ProviderCache};
use crate::azure_client_secret::region::{self, ConfiguredRegion, IMDS_REGION_URL};
use crate::azure_client_secret::transport::HttpMethod;
use crate::azure_client_secret::{
    ClientSecretError, ClientSecretInput, DEFAULT_SCOPE, PROVIDER_CACHE_CAPACITY,
    resolve_client_secret,
};

const TOKEN_BODY: &str = r#"{"access_token":"tok","expires_in":3600}"#;
const FIXTURE_SECRET: &str = "fixture-client-secret";

fn region_settings(regional: Option<&str>, forced: Option<&str>, region_name: &str) -> Settings {
    Settings {
        azure_regional_authority_name: regional.map(str::to_owned),
        msal_force_region: forced.map(str::to_owned),
        region_name: region_name.to_owned(),
    }
}

pub(super) fn client_secret_form() {
    form_request();
    form_redaction();
}

fn form_request() {
    let fixture = Fixture::new(vec![openid(HOST, "contoso"), token(TOKEN_BODY)]);
    let provider = fixture.provider_for(&input_for("contoso", CLIENT, FIXTURE_SECRET, None), None);
    let access = provider.token().unwrap();
    assert_eq!(access.token.expose_secret(), "tok");
    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, HttpMethod::Get);
    assert_eq!(
        requests[0].url,
        "https://login.microsoftonline.com/contoso/v2.0/.well-known/openid-configuration"
    );
    assert!(requests[0].query.is_empty() && requests[0].form.is_empty());
    let post = fixture.transport.posts().remove(0);
    assert_eq!(
        post.url,
        "https://login.microsoftonline.com/contoso/oauth2/v2.0/token"
    );
    let form = form_map(&post);
    assert_eq!(form.len(), 5);
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT));
    assert_eq!(
        form.get("client_secret").map(String::as_str),
        Some(FIXTURE_SECRET)
    );
    assert_eq!(form.get("client_info").map(String::as_str), Some("1"));
    assert_eq!(form.get("scope").map(String::as_str), Some(DEFAULT_SCOPE));
}

fn form_redaction() {
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(TOKEN_BODY)]);
    fixture.provider().token().unwrap();
    let post = fixture.transport.posts().remove(0);
    let debug = format!("{post:?}");
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains(FIXTURE_SECRET));
}

pub(super) fn tenant_scope() {
    explicit_scope();
    empty_scope();
    credential_errors();
}

fn explicit_scope() {
    let scope = "https://management.azure.com/.default";
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(TOKEN_BODY)]);
    fixture
        .provider_for(
            &input_for(TENANT, CLIENT, FIXTURE_SECRET, Some(scope)),
            None,
        )
        .token()
        .unwrap();
    let post = fixture.transport.posts().remove(0);
    assert!(post.url.contains(&format!("/{TENANT}/oauth2/v2.0/token")));
    assert_eq!(
        form_map(&post).get("scope").map(String::as_str),
        Some(scope)
    );
}

fn empty_scope() {
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(TOKEN_BODY)]);
    fixture
        .provider_for(&input_for(TENANT, CLIENT, FIXTURE_SECRET, Some("")), None)
        .token()
        .unwrap();
    let post = fixture.transport.posts().remove(0);
    assert_eq!(form_map(&post).get("scope").map(String::as_str), Some(""));
}

fn credential_errors() {
    let cases = [
        (
            input_for(TENANT, "", FIXTURE_SECRET, None),
            ClientSecretError::EmptyClientId,
        ),
        (
            input_for(TENANT, CLIENT, "", None),
            ClientSecretError::EmptyClientSecret,
        ),
        (
            input_for("", CLIENT, FIXTURE_SECRET, None),
            ClientSecretError::EmptyTenantId,
        ),
        (
            input_for("tenant name", CLIENT, FIXTURE_SECRET, None),
            ClientSecretError::InvalidTenantId,
        ),
    ];
    for (input, expected) in cases {
        let fixture = Fixture::new(Vec::new());
        let error = fixture
            .cache()
            .provider(&input, &fixture.resolver, None)
            .unwrap_err();
        assert_eq!(discriminant(&error), discriminant(&expected));
        assert!(fixture.transport.requests().is_empty());
    }
}

pub(super) fn oauth_failure() {
    let failure = Reply::status(
        400,
        "/oauth2/v2.0/token",
        r#"{"error":"invalid_client","error_description":"the secret is wrong"}"#,
    );
    let fixture = Fixture::new(vec![openid(HOST, TENANT), failure]);
    let error = fixture.provider().token().unwrap_err();
    assert!(matches!(error, ClientSecretError::TokenAccessTokenMissing));
    assert!(!error_text(&error).contains(FIXTURE_SECRET));
    assert_eq!(fixture.transport.posts().len(), 1);
    transport_failure();
    server_failure();
}

fn transport_failure() {
    let fixture = Fixture::new(vec![
        openid(HOST, TENANT),
        Reply::failure("/oauth2/v2.0/token"),
    ]);
    let error = fixture.provider().token().unwrap_err();
    assert!(matches!(error, ClientSecretError::TokenTransport { .. }));
    assert_eq!(error.to_string(), "Authentication failed: connection reset");
    assert_eq!(error.source().unwrap().to_string(), "connection reset");
}

fn server_failure() {
    let fixture = Fixture::new(vec![
        openid(HOST, TENANT),
        Reply::status(503, "/oauth2/v2.0/token", "unavailable"),
    ]);
    let error = fixture.provider().token().unwrap_err();
    assert!(matches!(error, ClientSecretError::Authentication { .. }));
    assert_eq!(error.to_string(), "Authentication failed: HTTP Error: 503");
    assert!(error.source().is_none());
}

pub(super) fn token_expiry() {
    token_lifetime();
    token_refresh();
    explicit_token_type();
    refresh_in_lifetime();
    stale_token_fallback();
    endpoint_resolved_once();
    failed_resolution_retries();
}

fn refresh_in_lifetime() {
    let body = r#"{"access_token":"tok","expires_in":3600,"refresh_in":1200}"#;
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(body)]);
    let access = fixture.provider().token().unwrap();
    assert_eq!(access.expires_on, NOW + 3600);
    assert_eq!(access.refresh_on, Some(NOW + 1200));
    refresh_in_wins();
    refresh_in_errors();
}

fn refresh_in_wins() {
    let both =
        r#"{"access_token":"tok","expires_in":3600,"refresh_in":1200,"refresh_on":"1700009999"}"#;
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(both)]);
    assert_eq!(
        fixture.provider().token().unwrap().refresh_on,
        Some(NOW + 1200)
    );
}

fn refresh_in_errors() {
    let text = r#"{"access_token":"tok","expires_in":3600,"refresh_in":"soon"}"#;
    let overflow = format!(
        r#"{{"access_token":"tok","expires_in":3600,"refresh_in":{}}}"#,
        i64::MAX
    );
    let errors: Vec<ClientSecretError> = [text, overflow.as_str()]
        .iter()
        .map(|body| {
            Fixture::new(vec![openid(HOST, TENANT), token(body)])
                .provider()
                .token()
                .unwrap_err()
        })
        .collect();
    assert!(matches!(
        errors[0],
        ClientSecretError::TokenInteger {
            field: "refresh_in"
        }
    ));
    assert!(matches!(errors[1], ClientSecretError::TokenExpiryOverflow));
}

fn stale_token_fallback() {
    let first = r#"{"access_token":"tok-1","expires_in":3600,"refresh_in":100}"#;
    let rejected = Reply::status(400, "/oauth2/v2.0/token", r#"{"error":"invalid_client"}"#);
    let fixture = Fixture::new(vec![
        openid(HOST, TENANT),
        token(first),
        rejected.clone(),
        rejected.clone(),
        rejected,
    ]);
    let provider = fixture.provider();
    provider.token().unwrap();
    fixture.clock.set(NOW + 100);
    let served = provider.token().unwrap();
    assert_eq!(served.token.expose_secret(), "tok-1");
    assert_eq!(served.expires_on, NOW + 3600);
    assert_eq!(fixture.transport.posts().len(), 2);
    fixture.clock.set(NOW + 3599);
    assert_eq!(provider.token().unwrap().token.expose_secret(), "tok-1");
    assert_eq!(fixture.transport.posts().len(), 3);
    fixture.clock.set(NOW + 3600);
    let error = provider.token().unwrap_err();
    assert!(matches!(error, ClientSecretError::TokenAccessTokenMissing));
    assert_eq!(fixture.transport.posts().len(), 4);
    unserved_failures(first);
}

// a transport or server failure while the token is still valid raises, as msal does
fn unserved_failures(first: &str) {
    for failure in [
        Reply::failure("/oauth2/v2.0/token"),
        Reply::status(503, "/oauth2/v2.0/token", "unavailable"),
    ] {
        let fixture = Fixture::new(vec![openid(HOST, TENANT), token(first), failure]);
        let provider = fixture.provider();
        provider.token().unwrap();
        fixture.clock.set(NOW + 100);
        let error = provider.token().unwrap_err();
        assert!(matches!(
            error,
            ClientSecretError::TokenTransport { .. } | ClientSecretError::Authentication { .. }
        ));
    }
}

fn endpoint_resolved_once() {
    let body = r#"{"access_token":"tok-1","expires_in":3600,"refresh_in":100}"#;
    let fixture = Fixture::new(vec![
        openid(HOST, TENANT),
        openid_for("westus2.login.microsoftonline.com"),
        token(body),
        token(TOKEN_BODY),
    ])
    .with_settings(region_settings(Some("true"), None, "westus2"));
    let provider = fixture.provider();
    provider.token().unwrap();
    let discoveries = fixture.transport.requests_matching("well-known").len();
    assert_eq!(discoveries, 2);
    fixture.clock.set(NOW + 100);
    assert_eq!(provider.token().unwrap().token.expose_secret(), "tok");
    assert_eq!(
        fixture.transport.requests_matching("well-known").len(),
        discoveries
    );
    let posts = fixture.transport.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].url, posts[1].url);
    assert!(posts[1].url.contains("westus2.login.microsoftonline.com"));
}

fn failed_resolution_retries() {
    let fixture = Fixture::new(vec![
        Reply::failure("well-known"),
        openid(HOST, TENANT),
        token(TOKEN_BODY),
    ]);
    let provider = fixture.provider();
    provider.token().unwrap_err();
    provider.token().unwrap();
    assert_eq!(fixture.transport.requests_matching("well-known").len(), 2);
}

fn token_lifetime() {
    let body = r#"{"access_token":"tok","expires_in":"3600","refresh_on":"1700003000"}"#;
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(body)]);
    let access = fixture.provider().token().unwrap();
    assert_eq!(access.expires_on, NOW + 3600);
    assert_eq!(access.token_type, "Bearer");
    assert_eq!(access.refresh_on, Some(1_700_003_000));
}

fn token_refresh() {
    let body = r#"{"access_token":"tok","expires_in":"3600","refresh_on":"1700003000"}"#;
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(body), token(body)]);
    let provider = fixture.provider();
    provider.token().unwrap();
    provider.token().unwrap();
    assert_eq!(fixture.transport.posts().len(), 1);
    fixture.clock.set(1_700_003_000);
    provider.token().unwrap();
    assert_eq!(fixture.transport.posts().len(), 2);
    assert_eq!(fixture.transport.requests_matching("well-known").len(), 1);
}

fn explicit_token_type() {
    let body = r#"{"access_token":"tok","expires_in":7200,"token_type":"Custom"}"#;
    let fixture = Fixture::new(vec![openid(HOST, TENANT), token(body)]);
    let access = fixture.provider().token().unwrap();
    assert_eq!(access.expires_on, NOW + 7200);
    assert_eq!(access.token_type, "Custom");
    assert_eq!(access.refresh_on, None);
}

pub(super) fn client_secret_reference_resolution() {
    resolves_references();
    literal_values();
    unresolved_reference();
}

fn resolves_references() {
    let resolver = FixtureResolver::new(&[
        ("TENANT", "resolved-tenant"),
        ("CLIENT", "resolved-client"),
        ("SECRET", "resolved-secret"),
    ]);
    let input = ClientSecretInput {
        tenant_id: "os.environ/TENANT".to_owned(),
        client_id: "os.environ/CLIENT".to_owned(),
        client_secret: SecretString::from("os.environ/SECRET"),
        scope: Some("custom-scope".to_owned()),
    };
    let credentials = resolve_client_secret(&input, &resolver).unwrap();
    assert_eq!(credentials.tenant_id, "resolved-tenant");
    assert_eq!(credentials.client_id, "resolved-client");
    assert_eq!(credentials.client_secret.expose_secret(), "resolved-secret");
    assert_eq!(credentials.scope, "custom-scope");
}

fn literal_values() {
    let resolver = FixtureResolver::new(&[]);
    let input = ClientSecretInput {
        tenant_id: String::new(),
        client_id: "literal-client".to_owned(),
        client_secret: SecretString::from(""),
        scope: None,
    };
    let credentials = resolve_client_secret(&input, &resolver).unwrap();
    assert!(credentials.tenant_id.is_empty());
    assert!(credentials.client_secret.expose_secret().is_empty());
    assert_eq!(credentials.scope, DEFAULT_SCOPE);
}

fn unresolved_reference() {
    let resolver = FixtureResolver::new(&[("BOOL", "true")]);
    let missing = ClientSecretInput {
        tenant_id: "os.environ/MISSING".to_owned(),
        client_id: CLIENT.to_owned(),
        client_secret: SecretString::from(SECRET),
        scope: None,
    };
    let error = resolve_client_secret(&missing, &resolver).unwrap_err();
    assert_eq!(
        error.to_string(),
        "tenant_id, client_id, and client_secret must be provided"
    );
    let unset = ClientSecretInput {
        client_secret: SecretString::from("os.environ/BOOL"),
        ..missing
    };
    assert!(matches!(
        resolve_client_secret(&unset, &resolver),
        Err(ClientSecretError::MissingCredentials)
    ));
}

pub(super) fn client_secret_provider_cache() {
    reuses_provider();
    isolates_keys();
    resolved_key();
    capacity();
}

fn reuses_provider() {
    let mut cache = Fixture::new(Vec::new()).cache();
    let resolver = FixtureResolver::new(&[]);
    let first = cache.provider(&input(), &resolver, None).unwrap();
    let second = cache.provider(&input(), &resolver, None).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

fn isolates_keys() {
    let mut cache = Fixture::new(Vec::new()).cache();
    let resolver = FixtureResolver::new(&[]);
    let base_provider = cache.provider(&input(), &resolver, None).unwrap();
    let variants = [
        input_for("other-tenant", CLIENT, SECRET, None),
        input_for(TENANT, "other-client", SECRET, None),
        input_for(TENANT, CLIENT, "other-secret", None),
        input_for(TENANT, CLIENT, SECRET, Some("other-scope")),
    ];
    for variant in &variants {
        assert_key_isolated(&mut cache, &resolver, &base_provider, variant);
    }
}

fn assert_key_isolated(
    cache: &mut ProviderCache,
    resolver: &FixtureResolver,
    base: &Arc<ClientSecretProvider>,
    variant: &ClientSecretInput,
) {
    let provider = cache.provider(variant, resolver, None).unwrap();
    assert!(!Arc::ptr_eq(base, &provider));
    assert!(Arc::ptr_eq(
        &provider,
        &cache.provider(variant, resolver, None).unwrap()
    ));
}

fn resolved_key() {
    let mut cache = Fixture::new(Vec::new()).cache();
    let resolver = FixtureResolver::new(&[]);
    let reference = FixtureResolver::new(&[("SECRET", "resolved-secret")]);
    let referenced = ClientSecretInput {
        client_secret: SecretString::from("os.environ/SECRET"),
        ..input()
    };
    let literal = input_for(TENANT, CLIENT, "resolved-secret", None);
    let via_reference = cache.provider(&referenced, &reference, None).unwrap();
    assert!(Arc::ptr_eq(
        &via_reference,
        &cache.provider(&literal, &resolver, None).unwrap()
    ));
    let rotated = cache
        .provider(
            &referenced,
            &FixtureResolver::new(&[("SECRET", "rotated-secret")]),
            None,
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&via_reference, &rotated));
}

fn capacity() {
    let mut cache = Fixture::new(Vec::new()).cache();
    let resolver = FixtureResolver::new(&[]);
    let first = cache
        .provider(
            &input_for("tenant-0", CLIENT, SECRET, None),
            &resolver,
            None,
        )
        .unwrap();
    for index in 1..=PROVIDER_CACHE_CAPACITY.get() {
        cache
            .provider(
                &input_for(&format!("tenant-{index}"), CLIENT, SECRET, None),
                &resolver,
                None,
            )
            .unwrap();
    }
    let evicted = cache
        .provider(
            &input_for("tenant-0", CLIENT, SECRET, None),
            &resolver,
            None,
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &evicted));
}

pub(super) fn azure_error_fields() {
    default_projection();
    supplied_projection();
}

fn default_projection() {
    let error = ProviderError::from(ProviderErrorParts {
        status_code: 400,
        message: "caller message".to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    });
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "caller message");
    assert_eq!(
        error.request,
        ErrorRequest {
            method: "POST".to_owned(),
            url: "https://docs.litellm.ai/docs".to_owned(),
        }
    );
    assert_eq!(
        error.response,
        ErrorResponse {
            status_code: 400,
            request: error.request.clone(),
        }
    );
    assert!(error.headers.is_none() && error.body.is_none());
    assert!(!error.status_code_is_synthesized);
}

fn supplied_projection() {
    let request = ErrorRequest {
        method: "GET".to_owned(),
        url: "https://example.test/resource".to_owned(),
    };
    let response = ErrorResponse {
        status_code: 401,
        request: request.clone(),
    };
    let headers = vec![("x-request-id".to_owned(), "abc".to_owned())];
    let error = ProviderError::from(ProviderErrorParts {
        status_code: 401,
        message: "supplied".to_owned(),
        headers: Some(headers.clone()),
        request: Some(request.clone()),
        response: Some(response.clone()),
        body: None,
        status_code_is_synthesized: true,
    });
    assert_eq!(error.request, request);
    assert_eq!(error.response, response);
    assert_eq!(error.headers, Some(headers));
    assert!(error.status_code_is_synthesized);
}

pub(super) fn regional_authority_inputs() {
    auto_detection_inputs();
    explicit_region();
    forced_region();
    empty_configured_region();
    no_configured_region();
    no_regional_inputs();
}

fn auto_detection_inputs() {
    for auto in ["true", "True", "tryautodetect", "TryAutoDetect"] {
        let transport = FixtureTransport::new(Vec::new());
        let selection =
            region::select_region(&region_settings(Some(auto), None, "westus2"), &transport);
        assert_eq!(
            selection.configured,
            Some(ConfiguredRegion::AutoDetect),
            "{auto}"
        );
        assert_eq!(selection.region.as_deref(), Some("westus2"));
        assert!(selection.warnings.is_empty());
        assert!(selection.opted_in);
    }
}

fn explicit_region() {
    let transport = FixtureTransport::new(Vec::new());
    let selection = region::select_region(
        &region_settings(Some("westus3"), Some("ignored"), " West US 2 "),
        &transport,
    );
    assert_eq!(selection.detected.as_deref(), Some("westus2"));
    assert_eq!(selection.region.as_deref(), Some("westus3"));
    assert_eq!(
        selection.warnings,
        vec!["Region configured ('westus3') != region detected ('westus2')".to_owned()]
    );
    assert_eq!(selection.region_debug, "Region to be used: 'westus3'");
    assert!(transport.requests().is_empty());
}

fn forced_region() {
    let transport = FixtureTransport::new(vec![imds("eastus\n")]);
    let selection = region::select_region(&region_settings(None, Some("eastus"), ""), &transport);
    assert_eq!(
        selection.configured,
        Some(ConfiguredRegion::Named("eastus".to_owned()))
    );
    assert_eq!(selection.region.as_deref(), Some("eastus"));
    assert!(selection.warnings.is_empty());
    let requests = transport.requests_matching("169.254.169.254");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .headers
            .contains(&("Metadata".to_owned(), "true".to_owned()))
    );
}

fn empty_configured_region() {
    let transport = FixtureTransport::new(vec![imds("westus2\n")]);
    let selection =
        region::select_region(&region_settings(Some(""), Some("ignored"), ""), &transport);
    assert_eq!(
        selection.configured,
        Some(ConfiguredRegion::Named(String::new()))
    );
    assert_eq!(selection.region.as_deref(), Some(""));
    assert_eq!(
        selection.warnings,
        vec!["Region configured ('') != region detected ('westus2')".to_owned()]
    );
    assert_eq!(selection.region_debug, "Region to be used: ''");
    assert!(!selection.opted_in);
    assert_eq!(transport.requests_matching("169.254.169.254").len(), 1);
}

fn no_configured_region() {
    let transport = FixtureTransport::new(Vec::new());
    let selection = region::select_region(&region_settings(None, None, "westus2"), &transport);
    assert_eq!(selection.configured, None);
    assert_eq!(selection.detected.as_deref(), Some("westus2"));
    assert_eq!(selection.region, None);
    assert_eq!(
        selection.warnings,
        vec!["Region configured (None) != region detected ('westus2')".to_owned()]
    );
    assert_eq!(selection.region_debug, "Region to be used: None");
    assert!(transport.requests().is_empty());
}

fn no_regional_inputs() {
    let transport = FixtureTransport::new(Vec::new());
    let selection = region::select_region(&region_settings(None, None, ""), &transport);
    assert_eq!(selection.detected, None);
    assert!(selection.warnings.is_empty());
    assert_eq!(selection.region_debug, "Region to be used: None");
    assert!(transport.requests().is_empty());
}

pub(super) fn regional_authority_discovery() {
    imds_request();
    imds_status_is_ignored();
    imds_failure();
    no_imds_without_configured_region();
    region_name_skips_imds();
}

fn imds_request() {
    let transport = FixtureTransport::new(vec![imds("  westus2 \n")]);
    let selection = region::select_region(&region_settings(Some("true"), None, ""), &transport);
    assert_eq!(selection.detected.as_deref(), Some("westus2"));
    assert_eq!(selection.region.as_deref(), Some("westus2"));
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, HttpMethod::Get);
    assert_eq!(requests[0].url, IMDS_REGION_URL);
    assert_eq!(
        requests[0].headers,
        vec![("Metadata".to_owned(), "true".to_owned())]
    );
    assert!(requests[0].query.is_empty() && requests[0].form.is_empty());
}

fn imds_status_is_ignored() {
    let transport = FixtureTransport::new(vec![Reply::status(500, "169.254.169.254", "westus2")]);
    let selection = region::select_region(&region_settings(Some("true"), None, ""), &transport);
    assert_eq!(selection.detected.as_deref(), Some("westus2"));
    assert_eq!(selection.region.as_deref(), Some("westus2"));
}

fn imds_failure() {
    let transport = FixtureTransport::new(vec![Reply::failure("169.254.169.254")]);
    let selection = region::select_region(&region_settings(Some("true"), None, ""), &transport);
    assert_eq!(selection.detected, None);
    assert_eq!(selection.region, None);
    assert_eq!(selection.region_debug, "Region to be used: None");
}

fn no_imds_without_configured_region() {
    let transport = FixtureTransport::new(Vec::new());
    let selection = region::select_region(&region_settings(None, None, ""), &transport);
    assert!(transport.requests().is_empty());
    assert_eq!(selection.region, None);
}

fn region_name_skips_imds() {
    let transport = FixtureTransport::new(Vec::new());
    let selection =
        region::select_region(&region_settings(Some("true"), None, "westus2"), &transport);
    assert!(transport.requests().is_empty());
    assert_eq!(selection.detected.as_deref(), Some("westus2"));
}

pub(super) fn regional_authority_host() {
    central_aliases();
    custom_cloud();
    fixed_instance_discovery();
    authority_errors();
    retry_policy();
}

fn central_aliases() {
    for central_host in [
        "login.microsoftonline.com",
        "login.microsoft.com",
        "login.windows.net",
        "sts.windows.net",
    ] {
        let fixture = Fixture::new(vec![openid_for("westus2.login.microsoft.com")]);
        let central = Authority {
            instance: central_host.to_owned(),
            tenant: TENANT.to_owned(),
            token_endpoint: format!("https://{central_host}/{TENANT}/oauth2/v2.0/token"),
        };
        let regional =
            authority::regional_authority(&*fixture.transport, &central, "westus2").unwrap();
        assert_eq!(regional.instance, "westus2.login.microsoft.com");
        let requests = fixture.transport.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.starts_with(
            "https://westus2.login.microsoft.com/tenant/v2.0/.well-known/openid-configuration"
        ));
    }
}

fn custom_cloud() {
    let fixture = Fixture::new(vec![openid_for("westus2.login.contoso.test")]);
    let central = Authority {
        instance: "login.contoso.test".to_owned(),
        tenant: TENANT.to_owned(),
        token_endpoint: "https://login.contoso.test/tenant/oauth2/v2.0/token".to_owned(),
    };
    let regional = authority::regional_authority(&*fixture.transport, &central, "westus2").unwrap();
    assert_eq!(regional.instance, "westus2.login.contoso.test");
    assert_eq!(
        regional.token_endpoint,
        "https://westus2.login.contoso.test/tenant/oauth2/v2.0/token"
    );
}

fn fixed_instance_discovery() {
    let fixture = Fixture::new(vec![
        instance_metadata(
            r#"{"tenant_discovery_endpoint":"https://login.contoso.test/tenant/v2.0/.well-known/openid-configuration"}"#,
        ),
        openid_for("login.contoso.test"),
    ]);
    let central = authority::discover_authority(
        &*fixture.transport,
        "https://login.contoso.test",
        TENANT,
        true,
    )
    .unwrap();
    assert_eq!(central.instance, "login.contoso.test");
    let requests = fixture.transport.requests();
    assert_eq!(
        requests[0].url,
        "https://login.microsoftonline.com/common/discovery/instance"
    );
    assert_eq!(
        requests[0].query,
        vec![
            (
                "authorization_endpoint".to_owned(),
                "https://login.contoso.test/tenant/oauth2/v2.0/authorize".to_owned()
            ),
            ("api-version".to_owned(), "1.0".to_owned())
        ]
    );
    assert_eq!(
        requests[1].url,
        "https://login.contoso.test/tenant/v2.0/.well-known/openid-configuration"
    );
}

fn authority_errors() {
    authority_value_errors();
    discovery_status_error();
    malformed_configuration();
}

fn authority_value_errors() {
    let unreachable = Fixture::new(Vec::new());
    let invalid = authority::discover_authority(
        &*unreachable.transport,
        "http://login.contoso.test",
        TENANT,
        true,
    )
    .unwrap_err();
    assert!(matches!(
        invalid,
        ClientSecretError::InvalidAuthority { .. }
    ));
    let fixture = Fixture::new(vec![instance_metadata(r#"{"error":"invalid_instance"}"#)]);
    let refused = authority::discover_authority(
        &*fixture.transport,
        "https://login.contoso.test",
        TENANT,
        true,
    )
    .unwrap_err();
    assert!(matches!(refused, ClientSecretError::InvalidInstance { .. }));
    assert_eq!(fixture.transport.requests().len(), 1);
}

fn discovery_status_error() {
    let fixture = Fixture::new(vec![Reply::status(404, "well-known", "no tenant")]);
    let error = authority::discover_authority(
        &*fixture.transport,
        "https://login.microsoftonline.com",
        "missing",
        true,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ClientSecretError::AuthorityConfiguration { .. }
    ));
}

fn malformed_configuration() {
    let fixture = Fixture::new(vec![Reply::ok("well-known", "not json")]);
    let error = authority::discover_authority(
        &*fixture.transport,
        "https://login.microsoftonline.com",
        TENANT,
        true,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ClientSecretError::AuthorityConfigurationJson { .. }
    ));
    assert!(error.is_authority_value());
    assert!(error.source().is_some());
}

fn retry_policy() {
    retry_with_region();
    no_retry_without_region();
    value_error_propagates();
}

fn retry_with_region() {
    let replies = vec![
        Reply::failure("/common/discovery/instance"),
        openid_for("login.contoso.test"),
        openid_for("westus2.login.contoso.test"),
        token(TOKEN_BODY),
    ];
    let fixture =
        Fixture::new(replies).with_settings(region_settings(Some("true"), None, "westus2"));
    let provider = fixture.provider_for(&input(), Some("https://login.contoso.test"));
    provider.token().unwrap();
    let requests = fixture.transport.requests();
    assert_eq!(
        requests[0].url,
        "https://login.microsoftonline.com/common/discovery/instance"
    );
    assert_eq!(
        requests[1].url,
        "https://login.contoso.test/tenant/v2.0/.well-known/openid-configuration"
    );
    assert_eq!(
        requests[2].url,
        "https://westus2.login.contoso.test/tenant/v2.0/.well-known/openid-configuration"
    );
    assert_eq!(
        fixture
            .transport
            .requests_matching("/common/discovery/instance")
            .len(),
        1
    );
}

fn no_retry_without_region() {
    let replies = vec![Reply::failure("/common/discovery/instance")];
    let fixture = Fixture::new(replies);
    let provider = fixture.provider_for(&input(), Some("https://login.contoso.test"));
    let error = provider.token().unwrap_err();
    assert!(matches!(
        error,
        ClientSecretError::AuthorityTransport { .. }
    ));
    assert_eq!(fixture.transport.requests().len(), 1);
}

fn value_error_propagates() {
    let fixture = Fixture::new(vec![instance_metadata(r#"{"error":"invalid_instance"}"#)])
        .with_settings(region_settings(Some("true"), None, "westus2"));
    let provider = fixture.provider_for(&input(), Some("https://login.contoso.test"));
    let error = provider.token().unwrap_err();
    assert!(matches!(error, ClientSecretError::InvalidInstance { .. }));
    assert_eq!(fixture.transport.requests().len(), 1);
}
