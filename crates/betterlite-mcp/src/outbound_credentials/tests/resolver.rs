use std::sync::Arc;

use rstest::rstest;

use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_config::aws::{AwsCredentialSource, AwsSigV4Config};
use crate::credential_contracts::auth_config::key_source::{ApiKeyConfig, ApiKeySource};
use crate::credential_contracts::auth_spec::AuthResolution;
use crate::credential_contracts::errors::{CredError, Unauthorized};
use crate::credential_contracts::header_carrier::HeaderCarrier;
use crate::credential_contracts::server_spec::ServerSpec;

use super::super::resolver::source::resolve_with_source;
use super::super::resolver::{Collaborators, SystemWallClock, WallClock as _};
use super::support::{
    FakeAssertions, FakeCredentials, FakeEndpoint, FakeExchanger, FakeTokens, applied, bearer,
    failure, header, id_jag_spec, param, provider, request, shared_credentials, shared_endpoint,
    shared_exchanger, spec, subject,
};

#[derive(serde::Serialize)]
struct ServerFixture {
    server_id: &'static str,
    name: &'static str,
    transport: &'static str,
    url: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authentication_token: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth2_flow: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_url: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_exchange_endpoint: Option<&'static str>,
}

const BASE_FIXTURE: ServerFixture = ServerFixture {
    server_id: "srv-1",
    name: "Server One",
    transport: "http",
    url: "https://mcp.example.com/mcp",
    auth_type: None,
    authentication_token: None,
    oauth2_flow: None,
    client_id: None,
    client_secret: None,
    token_url: None,
    token_exchange_endpoint: None,
};

#[derive(Clone, Copy)]
enum Oauth2Profile {
    ClientCredentials,
    TokenExchange,
}

fn oauth2_profile_spec(profile: Oauth2Profile) -> ServerSpec {
    let fixture = match profile {
        Oauth2Profile::ClientCredentials => ServerFixture {
            auth_type: Some("oauth2"),
            oauth2_flow: Some("client_credentials"),
            client_id: Some("client-1"),
            client_secret: Some("secret-1"),
            token_url: Some("https://idp.example.com/token"),
            ..BASE_FIXTURE
        },
        Oauth2Profile::TokenExchange => ServerFixture {
            auth_type: Some("oauth2_token_exchange"),
            token_exchange_endpoint: Some("https://idp.example.com/token"),
            client_id: Some("client-1"),
            client_secret: Some("secret-1"),
            ..BASE_FIXTURE
        },
    };
    spec(fixture)
}

fn none_spec(url: &'static str) -> ServerSpec {
    spec(ServerFixture {
        url,
        ..BASE_FIXTURE
    })
}

fn api_key_spec() -> ServerSpec {
    spec(ServerFixture {
        auth_type: Some("api_key"),
        authentication_token: Some("shared-token"),
        ..BASE_FIXTURE
    })
}

fn passthrough_spec() -> ServerSpec {
    spec(ServerFixture {
        auth_type: Some("true_passthrough"),
        ..BASE_FIXTURE
    })
}

fn authorization_code_spec() -> ServerSpec {
    spec(ServerFixture {
        auth_type: Some("oauth2"),
        ..BASE_FIXTURE
    })
}

#[tokio::test]
async fn none_config_no_userinfo() {
    let resolver = provider(Collaborators::default());
    let server = none_spec("https://mcp.example.com/mcp");
    let auth = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &server)
        .await
        .unwrap();
    assert!(auth.header_name().is_none());
    assert!(applied(auth.as_ref()).is_empty());
    let anonymous = resolver
        .resolve_credentials(&subject("", "", None), &server)
        .await
        .unwrap();
    assert!(applied(anonymous.as_ref()).is_empty());
}

#[rstest]
#[case::user_and_password("https://user:pass@mcp.example.com/mcp")]
#[case::user_only("https://user@mcp.example.com/mcp")]
#[tokio::test]
async fn none_config_rejects_userinfo(#[case] url: &'static str) {
    let resolver = provider(Collaborators::default());
    let error = failure(
        resolver
            .resolve_credentials(&subject("org-1", "user-1", None), &none_spec(url))
            .await,
    );
    assert_eq!(error, CredError::UrlCredentialsNotAllowed);
    let relative = none_spec("mcp.example.com/mcp");
    let auth = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &relative)
        .await
        .unwrap();
    assert!(auth.header_name().is_none());
}

#[tokio::test]
async fn api_key_static_source() {
    let resolver = provider(Collaborators::default());
    let auth = resolver
        .resolve_credentials(&subject("", "", None), &api_key_spec())
        .await
        .unwrap();
    assert_eq!(auth.header_name(), Some("X-API-Key"));
    assert_eq!(header(&auth, "X-API-Key").as_deref(), Some("shared-token"));
}

#[tokio::test]
async fn api_key_replaces_its_slot_and_keeps_other_headers() {
    let auth = provider(Collaborators::default())
        .resolve_credentials(&subject("", "", None), &api_key_spec())
        .await
        .unwrap();
    let mut outgoing = request();
    outgoing
        .headers
        .insert("x-api-key".to_owned(), "stale".to_owned());
    outgoing
        .headers
        .insert("X-Trace".to_owned(), "keep".to_owned());
    let applied = auth.authenticate(outgoing).unwrap();
    assert_eq!(
        applied.headers.get("X-API-Key").map(String::as_str),
        Some("shared-token")
    );
    assert_eq!(
        applied.headers.get("X-Trace").map(String::as_str),
        Some("keep")
    );
    assert!(!applied.headers.contains_key("x-api-key"));
}

#[tokio::test]
async fn api_key_byok_source_is_not_implemented() {
    let config = ApiKeyConfig {
        header: HeaderCarrier::new("X-API-Key", "").unwrap(),
        key_source: ApiKeySource::Byok,
    };
    let server = ServerSpec {
        server_id: "srv-1".to_owned(),
        resource: "https://mcp.example.com/mcp".to_owned(),
        config: AuthConfig::ApiKey(config),
    };
    let error = failure(
        provider(Collaborators::default())
            .resolve_credentials(&subject("org-1", "user-1", None), &server)
            .await,
    );
    let expected = CredError::NotImplemented("api_key BYOK source not implemented yet".to_owned());
    assert_eq!(error, expected);
}

#[tokio::test]
async fn passthrough_present_token() {
    let resolver = provider(Collaborators::default());
    let auth = resolver
        .resolve_credentials(
            &subject("org-1", "user-1", Some("Bearer caller-token")),
            &passthrough_spec(),
        )
        .await
        .unwrap();
    assert_eq!(auth.header_name(), Some("Authorization"));
    assert_eq!(
        header(&auth, "Authorization").as_deref(),
        Some("Bearer caller-token")
    );
}

#[tokio::test]
async fn passthrough_missing_token() {
    let resolver = provider(Collaborators::default());
    let auth = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &passthrough_spec())
        .await
        .unwrap();
    assert!(auth.header_name().is_none());
    assert!(applied(auth.as_ref()).is_empty());
}

#[tokio::test]
async fn client_credentials_success() {
    let source = Arc::new(FakeCredentials::new("minted", Some("fresh")));
    let collaborators = Collaborators::default()
        .with_client_credentials_source(shared_credentials(Arc::clone(&source)));
    let auth = provider(collaborators)
        .resolve_credentials(
            &subject("", "", None),
            &oauth2_profile_spec(Oauth2Profile::ClientCredentials),
        )
        .await
        .unwrap();
    assert_eq!(auth.header_name(), Some("Authorization"));
    assert_eq!(bearer(&auth), "minted");
    assert!(auth.after_unauthorized().await);
    assert_eq!(bearer(&auth), "fresh");
    assert_eq!(source.refetched_tokens(), vec!["minted".to_owned()]);
}

#[tokio::test]
async fn client_credentials_keeps_the_rejected_token_without_a_refetch() {
    let source = Arc::new(FakeCredentials::new("minted", None));
    let collaborators = Collaborators::default()
        .with_client_credentials_source(shared_credentials(Arc::clone(&source)));
    let auth = provider(collaborators)
        .resolve_credentials(
            &subject("", "", None),
            &oauth2_profile_spec(Oauth2Profile::ClientCredentials),
        )
        .await
        .unwrap();
    assert!(!auth.after_unauthorized().await);
    assert_eq!(bearer(&auth), "minted");
    assert_eq!(source.refetched_tokens(), vec!["minted".to_owned()]);
}

#[tokio::test]
async fn client_credentials_without_a_source_fails_closed() {
    let error = failure(
        provider(Collaborators::default())
            .resolve_credentials(
                &subject("", "", None),
                &oauth2_profile_spec(Oauth2Profile::ClientCredentials),
            )
            .await,
    );
    let expected = CredError::Misconfigured("client_credentials collaborator not wired".to_owned());
    assert_eq!(error, expected);
}

#[tokio::test]
async fn token_exchange_no_inbound_fails_closed() {
    let error = failure(
        provider(Collaborators::default())
            .resolve_credentials(
                &subject("org-1", "user-1", None),
                &oauth2_profile_spec(Oauth2Profile::TokenExchange),
            )
            .await,
    );
    let expected = CredError::Unauthorized(Unauthorized {
        detail: "Token exchange requires a caller token to exchange (OBO).".to_owned(),
        www_authenticate: Some("Bearer error=\"invalid_request\"".to_owned()),
        body: None,
        claims: None,
    });
    assert_eq!(error, expected);
}

#[tokio::test]
async fn token_exchange_reaches_the_exchanger() {
    let exchanger = Arc::new(FakeExchanger::returning("exchanged-token"));
    let collaborators =
        Collaborators::default().with_token_exchanger(shared_exchanger(Arc::clone(&exchanger)));
    let caller = subject("org-1", "user-1", Some("caller-token"));
    let auth = provider(collaborators)
        .resolve_credentials(&caller, &oauth2_profile_spec(Oauth2Profile::TokenExchange))
        .await
        .unwrap();
    assert_eq!(bearer(&auth), "exchanged-token");
    assert_eq!(
        exchanger.exchanges(),
        vec![("caller-token".to_owned(), "org-1".to_owned())]
    );
}

#[tokio::test]
async fn token_exchange_propagates_the_exchanger_error() {
    let exchanger = Arc::new(FakeExchanger::failing(CredError::UpstreamUnavailable(
        "idp down".to_owned(),
    )));
    let collaborators = Collaborators::default().with_token_exchanger(exchanger);
    let error = failure(
        provider(collaborators)
            .resolve_credentials(
                &subject("org-1", "user-1", Some("caller-token")),
                &oauth2_profile_spec(Oauth2Profile::TokenExchange),
            )
            .await,
    );
    assert_eq!(error, CredError::UpstreamUnavailable("idp down".to_owned()));
}

#[tokio::test]
async fn authorization_code_success() {
    let store = Arc::new(FakeTokens::with_token("user-1", "user-token"));
    let resolver = provider(Collaborators::default().with_oauth_token_store(store));
    let caller = subject("org-1", "user-1", None);
    let auth = resolver
        .resolve_credentials(&caller, &authorization_code_spec())
        .await
        .unwrap();
    assert_eq!(bearer(&auth), "user-token");
    assert!(
        resolver
            .has_user_token(&caller, &authorization_code_spec())
            .await
    );
    let stranger = subject("org-1", "user-2", None);
    assert!(
        !resolver
            .has_user_token(&stranger, &authorization_code_spec())
            .await
    );
}

#[rstest]
#[case::empty(FakeTokens::empty())]
#[case::unavailable(FakeTokens::unavailable())]
#[tokio::test]
async fn authorization_code_missing_token_fails_closed(#[case] store: FakeTokens) {
    let expected = CredError::Unauthorized(Unauthorized {
        detail: "Authorization required: complete the OAuth flow for this server.".to_owned(),
        www_authenticate: None,
        body: None,
        claims: None,
    });
    let resolver = provider(Collaborators::default().with_oauth_token_store(Arc::new(store)));
    let caller = subject("org-1", "user-1", None);
    let error = failure(
        resolver
            .resolve_credentials(&caller, &authorization_code_spec())
            .await,
    );
    assert_eq!(error, expected);
    assert!(
        !resolver
            .has_user_token(&caller, &authorization_code_spec())
            .await
    );
}

const TOKEN_EXCHANGE_GRANT: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ID_JAG_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id-jag";
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

#[tokio::test]
async fn id_jag_two_leg_exchange() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["leg1-jag", "leg2-bearer"]));
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
        .with_token_endpoint(shared_endpoint(Arc::clone(&endpoint)));
    let resolver = provider(collaborators);
    let server = id_jag_spec("secret-1");
    let caller = subject("org-1", "user-1", None);
    let auth = resolver
        .resolve_credentials(&caller, &server)
        .await
        .unwrap();
    assert_eq!(bearer(&auth), "leg2-bearer");
    let calls = endpoint.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].endpoint, "https://idp.example.com/org/token");
    assert_eq!(calls[0].client_id, "client-1");
    assert_eq!(param(&calls[0], "grant_type"), Some(TOKEN_EXCHANGE_GRANT));
    assert_eq!(
        param(&calls[0], "requested_token_type"),
        Some(ID_JAG_TOKEN_TYPE)
    );
    assert_eq!(param(&calls[0], "subject_token"), Some("id-token"));
    assert_eq!(param(&calls[0], "scope"), Some("read write"));
    assert_eq!(param(&calls[0], "audience"), Some("api://upstream"));
    assert_eq!(calls[1].endpoint, "https://upstream.example.com/token");
    assert_eq!(param(&calls[1], "grant_type"), Some(JWT_BEARER_GRANT));
    assert_eq!(param(&calls[1], "assertion"), Some("leg1-jag"));
    let cached = resolver
        .resolve_credentials(&caller, &server)
        .await
        .unwrap();
    assert_eq!(bearer(&cached), "leg2-bearer");
    assert_eq!(endpoint.calls().len(), 2);
}

#[tokio::test]
async fn aws_sigv4_arm_is_not_implemented() {
    let config = AwsSigV4Config {
        region: "us-east-1".to_owned(),
        service: "bedrock-agentcore".to_owned(),
        credentials: AwsCredentialSource::Ambient,
    };
    let server = ServerSpec {
        server_id: "srv-1".to_owned(),
        resource: "https://bedrock-agentcore.example.com".to_owned(),
        config: AuthConfig::AwsSigv4(config),
    };
    let error = failure(
        provider(Collaborators::default())
            .resolve_credentials(&subject("org-1", "user-1", None), &server)
            .await,
    );
    let expected =
        CredError::NotImplemented("aws_sigv4: resolver arm not implemented yet".to_owned());
    assert_eq!(error, expected);
}

#[rstest]
#[case::none(
    none_spec("https://mcp.example.com/mcp"),
    AuthResolution::NoAuth,
    Some("caller-token")
)]
#[case::api_key(api_key_spec(), AuthResolution::StaticToken, Some("caller-token"))]
#[case::passthrough(
    passthrough_spec(),
    AuthResolution::Oauth2Passthrough,
    Some("caller-token")
)]
#[case::authorization_code(
    authorization_code_spec(),
    AuthResolution::StoredUserToken,
    Some("caller-token")
)]
#[case::client_credentials(
    oauth2_profile_spec(Oauth2Profile::ClientCredentials),
    AuthResolution::ClientCredentials,
    Some("caller-token")
)]
#[case::token_exchange(
    oauth2_profile_spec(Oauth2Profile::TokenExchange),
    AuthResolution::TokenExchange,
    Some("caller-token")
)]
#[case::id_jag(id_jag_spec("secret-1"), AuthResolution::IdJag, Some("caller-token"))]
#[case::anonymous_passthrough(passthrough_spec(), AuthResolution::NoAuth, None)]
#[tokio::test]
async fn resolve_credentials_with_source_names_each_mode(
    #[case] server: ServerSpec,
    #[case] expected: AuthResolution,
    #[case] inbound_token: Option<&str>,
) {
    let collaborators = Collaborators::default()
        .with_oauth_token_store(Arc::new(FakeTokens::with_token("user-1", "user-token")))
        .with_token_exchanger(Arc::new(FakeExchanger::returning("exchanged-token")))
        .with_client_credentials_source(Arc::new(FakeCredentials::new("minted", None)))
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
        .with_token_endpoint(Arc::new(FakeEndpoint::tokens(&["leg1-jag", "leg2-bearer"])));
    let resolver = provider(collaborators);
    let caller = subject("org-1", "user-1", inbound_token);
    let resolved = resolve_with_source(&resolver, &caller, &server)
        .await
        .unwrap();
    assert_eq!(resolved.source, expected);
}

#[tokio::test]
async fn invalidate_id_jag_by_slot() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["one", "two", "three", "four"]));
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
        .with_token_endpoint(shared_endpoint(Arc::clone(&endpoint)));
    let resolver = provider(collaborators);
    let server = id_jag_spec("secret-1");
    let alice = subject("org-1", "user-1", None);
    let bob = subject("org-1", "user-2", None);
    let first = resolver.resolve_credentials(&alice, &server).await.unwrap();
    assert_eq!(bearer(&first), "two");
    resolver.invalidate_credentials(&bob, &server).await;
    let cached = resolver.resolve_credentials(&alice, &server).await.unwrap();
    assert_eq!(bearer(&cached), "two");
    assert_eq!(endpoint.calls().len(), 2);
    resolver.invalidate_credentials(&alice, &server).await;
    let minted = resolver.resolve_credentials(&alice, &server).await.unwrap();
    assert_eq!(bearer(&minted), "four");
    assert_eq!(endpoint.calls().len(), 4);
}

#[tokio::test]
async fn invalidate_token_exchange_drops_bearer() {
    let exchanger = Arc::new(FakeExchanger::returning("exchanged-token"));
    let collaborators =
        Collaborators::default().with_token_exchanger(shared_exchanger(Arc::clone(&exchanger)));
    let resolver = provider(collaborators);
    let server = oauth2_profile_spec(Oauth2Profile::TokenExchange);
    let caller = subject("org-1", "user-1", Some("caller-token"));
    let _before = resolver
        .resolve_credentials(&caller, &server)
        .await
        .unwrap();
    resolver.invalidate_credentials(&caller, &server).await;
    assert_eq!(
        exchanger.invalidations(),
        vec![("caller-token".to_owned(), "org-1".to_owned())]
    );
    let _after = resolver
        .resolve_credentials(&caller, &server)
        .await
        .unwrap();
    assert_eq!(exchanger.exchanges().len(), 2);
}

#[rstest]
#[case::none(none_spec("https://mcp.example.com/mcp"))]
#[case::api_key(api_key_spec())]
#[case::passthrough(passthrough_spec())]
#[case::authorization_code(authorization_code_spec())]
#[case::client_credentials(oauth2_profile_spec(Oauth2Profile::ClientCredentials))]
#[tokio::test]
async fn invalidate_nonexchanged_noop(#[case] server: ServerSpec) {
    let exchanger = Arc::new(FakeExchanger::returning("exchanged-token"));
    let source = Arc::new(FakeCredentials::new("minted", Some("fresh")));
    let collaborators = Collaborators::default()
        .with_token_exchanger(shared_exchanger(Arc::clone(&exchanger)))
        .with_client_credentials_source(shared_credentials(Arc::clone(&source)))
        .with_oauth_token_store(Arc::new(FakeTokens::with_token("user-1", "user-token")));
    let resolver = provider(collaborators);
    let caller = subject("org-1", "user-1", Some("caller-token"));
    resolver.invalidate_credentials(&caller, &server).await;
    assert!(exchanger.invalidations().is_empty());
    assert!(source.refetched_tokens().is_empty());
    assert!(exchanger.exchanges().is_empty());
}

#[tokio::test]
async fn invalidate_with_no_inbound_noop() {
    let exchanger = Arc::new(FakeExchanger::returning("exchanged-token"));
    let collaborators =
        Collaborators::default().with_token_exchanger(shared_exchanger(Arc::clone(&exchanger)));
    let resolver = provider(collaborators);
    let anonymous = subject("org-1", "user-1", None);
    resolver
        .invalidate_credentials(
            &anonymous,
            &oauth2_profile_spec(Oauth2Profile::TokenExchange),
        )
        .await;
    assert!(exchanger.invalidations().is_empty());
    assert!(exchanger.exchanges().is_empty());
}

#[test]
fn the_system_clock_reads_the_wall_time() {
    let clock = SystemWallClock;
    assert!(clock.now().timestamp() >= 1_700_000_000);
}
