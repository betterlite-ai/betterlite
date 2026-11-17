use betterlite_error::proxy_error_types::ProxyErrorTypes;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::oauth_client::OAuthClient;
use crate::oauth_client::document::OAuthDiscoveryDocument;
use crate::oauth_client::error::{OAuthClientError, OAuthEndpoint, OAuthProviderFailure};
use crate::oauth_client::exchange::{CallbackRequest, ExchangeRequest, OAuthExchange, exchange};
use crate::oauth_client::tests::provider::transport;

fn client(allow_insecure_http: bool) -> OAuthClient {
    OAuthClient {
        client_id: "client-id".to_owned(),
        client_secret: SecretString::from("client-secret"),
        redirect_uri: Some("https://proxy.test/cb".to_owned()),
        scopes: vec!["openid".to_owned()],
        allow_insecure_http,
        additional_headers: HeaderMap::new(),
    }
}

fn discovery(token_endpoint: &str) -> OAuthDiscoveryDocument {
    OAuthDiscoveryDocument {
        authorization_endpoint: "https://provider.test/authorize".to_owned(),
        token_endpoint: token_endpoint.to_owned(),
        userinfo_endpoint: "https://provider.test/userinfo".to_owned(),
    }
}

fn request<'callback>(
    url: &'callback str,
    sso_state_cookie: Option<&'callback str>,
) -> ExchangeRequest<'callback> {
    ExchangeRequest {
        callback: CallbackRequest {
            url,
            sso_state_cookie,
        },
        redirect_uri: None,
        include_client_id: true,
    }
}

async fn exchanged(
    url: &str,
    sso_state_cookie: Option<&str>,
) -> Result<OAuthExchange, OAuthClientError> {
    exchange(
        &client(true),
        &transport(),
        &discovery(""),
        &request(url, sso_state_cookie),
    )
    .await
}

fn status(result: Result<OAuthExchange, OAuthClientError>) -> Option<u16> {
    result.err().map(|error| error.response().status)
}

#[tokio::test]
async fn a_callback_without_a_code_is_a_bad_request() {
    let client = client(true);
    let transport = transport();
    let discovery = discovery("");
    let absent = exchange(
        &client,
        &transport,
        &discovery,
        &request("https://proxy.test/cb?state=one", None),
    )
    .await;
    assert!(matches!(&absent, Err(OAuthClientError::MissingCode)));
    assert_eq!(status(absent), Some(400));
    let blank = exchange(
        &client,
        &transport,
        &discovery,
        &request("https://proxy.test/cb?code=&state=one", None),
    )
    .await;
    assert_eq!(status(blank), Some(400));
}

#[tokio::test]
async fn a_callback_url_that_is_not_a_url_is_a_server_error() {
    let error = exchange(
        &client(true),
        &transport(),
        &discovery(""),
        &request("/sso/callback?code=abc", None),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(&error, OAuthClientError::CallbackUrl(_)));
    assert_eq!(error.response().status, 500);
}

#[tokio::test]
async fn a_state_cookie_must_match_a_supplied_state() {
    let mismatched = exchanged("https://proxy.test/cb?code=abc&state=one", Some("two")).await;
    assert!(matches!(&mismatched, Err(OAuthClientError::InvalidState)));
    let response = mismatched.err().map(|error| error.response()).unwrap();
    assert_eq!((response.status, response.param), (401, Some("state")));
}

#[tokio::test]
async fn a_matching_state_a_missing_cookie_or_a_missing_state_is_accepted() {
    let matched = exchanged("https://proxy.test/cb?code=abc&state=one", Some("one")).await;
    assert!(matches!(
        &matched,
        Err(OAuthClientError::Endpoint {
            endpoint: OAuthEndpoint::Token,
            ..
        })
    ));
    let no_cookie = exchanged("https://proxy.test/cb?code=abc&state=one", None).await;
    assert!(matches!(
        &no_cookie,
        Err(OAuthClientError::Endpoint {
            endpoint: OAuthEndpoint::Token,
            ..
        })
    ));
    let no_state = exchanged("https://proxy.test/cb?code=abc", Some("two")).await;
    assert!(matches!(
        &no_state,
        Err(OAuthClientError::Endpoint {
            endpoint: OAuthEndpoint::Token,
            ..
        })
    ));
}

#[tokio::test]
async fn a_provider_error_needs_a_code_and_maps_to_an_auth_failure() {
    let client = client(true);
    let transport = transport();
    let discovery = discovery("");
    let without_code = exchange(
        &client,
        &transport,
        &discovery,
        &request("https://proxy.test/cb?error=access_denied", None),
    )
    .await;
    assert!(matches!(&without_code, Err(OAuthClientError::MissingCode)));
    let error = exchange(
        &client,
        &transport,
        &discovery,
        &request(
            "https://proxy.test/cb?code=abc&error=access_denied&error_description=Denied",
            None,
        ),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error.response().status, 401);
    let OAuthClientError::Provider {
        error: provider_error,
        description,
    } = error
    else {
        panic!("the callback error is a provider error");
    };
    assert_eq!(provider_error, "access_denied");
    assert_eq!(description.as_deref(), Some("Denied"));
}

#[tokio::test]
async fn an_insecure_token_endpoint_is_a_server_error() {
    let error = exchange(
        &client(false),
        &transport(),
        &discovery("http://provider.test/token"),
        &request("https://proxy.test/cb?code=abc", None),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(
        &error,
        OAuthClientError::Insecure {
            endpoint: OAuthEndpoint::Token
        }
    ));
    assert_eq!(error.response().status, 500);
}

#[rstest]
#[case::missing_code(
    OAuthClientError::MissingCode,
    (400, ProxyErrorTypes::AuthError, Some("code"), "400")
)]
#[case::invalid_state(
    OAuthClientError::InvalidState,
    (401, ProxyErrorTypes::AuthError, Some("state"), "401")
)]
#[case::provider_refusal(
    OAuthClientError::Provider { error: "access_denied".to_owned(), description: None },
    (401, ProxyErrorTypes::AuthError, None, "401")
)]
#[case::insecure(
    OAuthClientError::Insecure { endpoint: OAuthEndpoint::Token },
    (500, ProxyErrorTypes::InternalServerError, None, "500")
)]
#[case::discovery(
    OAuthClientError::Discovery(OAuthProviderFailure::Status { status: 500 }),
    (503, ProxyErrorTypes::AuthProviderUnavailable, None, "503")
)]
#[case::missing_access_token(
    OAuthClientError::Token(OAuthProviderFailure::MissingAccessToken),
    (401, ProxyErrorTypes::AuthError, None, "401")
)]
#[case::userinfo(
    OAuthClientError::Userinfo(OAuthProviderFailure::Status { status: 401 }),
    (401, ProxyErrorTypes::AuthError, None, "401")
)]
fn oauth_failures_map_to_proxy_error_responses(
    #[case] error: OAuthClientError,
    #[case] expected: (u16, ProxyErrorTypes, Option<&'static str>, &'static str),
) {
    let response = error.response();
    assert_eq!(
        (
            response.status,
            response.error_type,
            response.param,
            response.code
        ),
        expected
    );
}

#[rstest]
#[case::discovery(OAuthEndpoint::Discovery, "discovery")]
#[case::authorization(OAuthEndpoint::Authorization, "authorization")]
#[case::token(OAuthEndpoint::Token, "token")]
#[case::userinfo(OAuthEndpoint::Userinfo, "userinfo")]
fn an_endpoint_error_names_the_endpoint(
    #[case] endpoint: OAuthEndpoint,
    #[case] name: &'static str,
) {
    assert_eq!(endpoint.name(), name);
    let error = OAuthClientError::Insecure { endpoint };
    assert_eq!(
        error.to_string(),
        format!("the {name} endpoint is not HTTPS and allow_insecure_http is off")
    );
}
