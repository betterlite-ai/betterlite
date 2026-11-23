use std::sync::Arc;

use serde_json::json;

use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_config::aws::{AwsCredentialSource, AwsSigV4Config};
use crate::credential_contracts::auth_config::key_source::{ApiKeyConfig, ApiKeySource};
use crate::credential_contracts::auth_spec::AuthResolution;
use crate::credential_contracts::errors::{CredError, Unauthorized};
use crate::credential_contracts::header_carrier::HeaderCarrier;
use crate::credential_contracts::server_spec::ServerSpec;

use super::super::resolver::Collaborators;
use super::super::resolver::source::resolve_credentials_with_source;
use super::support::{
    FakeAssertions, FakeCredentials, FakeEndpoint, FakeExchanger, FakeTokens, applied, bearer,
    failure, header, id_jag_spec, param, provider, spec, subject,
};

fn none_spec(url: &str) -> ServerSpec {
    spec(json!({
        "server_id": "srv-1",
        "name": "Server One",
        "transport": "http",
        "url": url
    }))
}

fn api_key_spec() -> ServerSpec {
    spec(json!({
        "server_id": "srv-1",
        "name": "Server One",
        "transport": "http",
        "url": "https://mcp.example.com/mcp",
        "auth_type": "api_key",
        "authentication_token": "shared-token"
    }))
}

fn passthrough_spec() -> ServerSpec {
    spec(json!({
        "server_id": "srv-1",
        "name": "Server One",
        "transport": "http",
        "url": "https://mcp.example.com/mcp",
        "auth_type": "true_passthrough"
    }))
}

fn authorization_code_spec() -> ServerSpec {
    spec(json!({
        "server_id": "srv-1",
        "name": "Server One",
        "transport": "http",
        "url": "https://mcp.example.com/mcp",
        "auth_type": "oauth2"
    }))
}

fn client_credentials_spec() -> ServerSpec {
    spec(json!({
        "server_id": "srv-1",
        "name": "Server One",
        "transport": "http",
        "url": "https://mcp.example.com/mcp",
        "auth_type": "oauth2",
        "oauth2_flow": "client_credentials",
        "client_id": "client-1",
        "client_secret": "secret-1",
        "token_url": "https://idp.example.com/token"
    }))
}

fn token_exchange_spec() -> ServerSpec {
    spec(json!({
        "server_id": "srv-1",
        "name": "Server One",
        "transport": "http",
        "url": "https://mcp.example.com/mcp",
        "auth_type": "oauth2_token_exchange",
        "token_exchange_endpoint": "https://idp.example.com/token",
        "client_id": "client-1",
        "client_secret": "secret-1"
    }))
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

#[tokio::test]
async fn none_config_rejects_userinfo() {
    let resolver = provider(Collaborators::default());
    for url in [
        "https://user:pass@mcp.example.com/mcp",
        "https://user@mcp.example.com/mcp",
    ] {
        let error = failure(
            resolver
                .resolve_credentials(&subject("org-1", "user-1", None), &none_spec(url))
                .await,
        );
        assert_eq!(error, CredError::UrlCredentialsNotAllowed);
    }
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
    let collaborators = Collaborators::default().with_client_credentials_source(source.clone());
    let auth = provider(collaborators)
        .resolve_credentials(&subject("", "", None), &client_credentials_spec())
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
    let collaborators = Collaborators::default().with_client_credentials_source(source.clone());
    let auth = provider(collaborators)
        .resolve_credentials(&subject("", "", None), &client_credentials_spec())
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
            .resolve_credentials(&subject("", "", None), &client_credentials_spec())
            .await,
    );
    let expected = CredError::Misconfigured("client_credentials collaborator not wired".to_owned());
    assert_eq!(error, expected);
}

#[tokio::test]
async fn token_exchange_no_inbound_fails_closed() {
    let error = failure(
        provider(Collaborators::default())
            .resolve_credentials(&subject("org-1", "user-1", None), &token_exchange_spec())
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
    let collaborators = Collaborators::default().with_token_exchanger(exchanger.clone());
    let caller = subject("org-1", "user-1", Some("caller-token"));
    let auth = provider(collaborators)
        .resolve_credentials(&caller, &token_exchange_spec())
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
                &token_exchange_spec(),
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

#[tokio::test]
async fn authorization_code_missing_token_fails_closed() {
    let expected = CredError::Unauthorized(Unauthorized {
        detail: "Authorization required: complete the OAuth flow for this server.".to_owned(),
        www_authenticate: None,
        body: None,
        claims: None,
    });
    for store in [FakeTokens::empty(), FakeTokens::unavailable()] {
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
}

const TOKEN_EXCHANGE_GRANT: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ID_JAG_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id-jag";
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

#[tokio::test]
async fn id_jag_two_leg_exchange() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["leg1-jag", "leg2-bearer"]));
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
        .with_token_endpoint(endpoint.clone());
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

#[tokio::test]
async fn resolve_credentials_with_source_names_each_mode() {
    let collaborators = Collaborators::default()
        .with_oauth_token_store(Arc::new(FakeTokens::with_token("user-1", "user-token")))
        .with_token_exchanger(Arc::new(FakeExchanger::returning("exchanged-token")))
        .with_client_credentials_source(Arc::new(FakeCredentials::new("minted", None)))
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
        .with_token_endpoint(Arc::new(FakeEndpoint::tokens(&["leg1-jag", "leg2-bearer"])));
    let resolver = provider(collaborators);
    let caller = subject("org-1", "user-1", Some("caller-token"));
    let cases = [
        (
            none_spec("https://mcp.example.com/mcp"),
            AuthResolution::NoAuth,
        ),
        (api_key_spec(), AuthResolution::StaticToken),
        (passthrough_spec(), AuthResolution::Oauth2Passthrough),
        (authorization_code_spec(), AuthResolution::StoredUserToken),
        (client_credentials_spec(), AuthResolution::ClientCredentials),
        (token_exchange_spec(), AuthResolution::TokenExchange),
        (id_jag_spec("secret-1"), AuthResolution::IdJag),
    ];
    for (server, expected) in cases {
        let resolved = resolve_credentials_with_source(&resolver, &caller, &server)
            .await
            .unwrap();
        assert_eq!(resolved.source, expected);
    }
    let anonymous = subject("org-1", "user-1", None);
    let resolved = resolve_credentials_with_source(&resolver, &anonymous, &passthrough_spec())
        .await
        .unwrap();
    assert_eq!(resolved.source, AuthResolution::NoAuth);
}

#[tokio::test]
async fn invalidate_id_jag_by_slot() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["one", "two", "three", "four"]));
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
        .with_token_endpoint(endpoint.clone());
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
    let collaborators = Collaborators::default().with_token_exchanger(exchanger.clone());
    let resolver = provider(collaborators);
    let server = token_exchange_spec();
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

#[tokio::test]
async fn invalidate_nonexchanged_noop() {
    let exchanger = Arc::new(FakeExchanger::returning("exchanged-token"));
    let source = Arc::new(FakeCredentials::new("minted", Some("fresh")));
    let collaborators = Collaborators::default()
        .with_token_exchanger(exchanger.clone())
        .with_client_credentials_source(source.clone())
        .with_oauth_token_store(Arc::new(FakeTokens::with_token("user-1", "user-token")));
    let resolver = provider(collaborators);
    let caller = subject("org-1", "user-1", Some("caller-token"));
    let servers = [
        none_spec("https://mcp.example.com/mcp"),
        api_key_spec(),
        passthrough_spec(),
        authorization_code_spec(),
        client_credentials_spec(),
    ];
    for server in servers {
        resolver.invalidate_credentials(&caller, &server).await;
    }
    assert!(exchanger.invalidations().is_empty());
    assert!(source.refetched_tokens().is_empty());
    assert!(exchanger.exchanges().is_empty());
}

#[tokio::test]
async fn invalidate_with_no_inbound_noop() {
    let exchanger = Arc::new(FakeExchanger::returning("exchanged-token"));
    let collaborators = Collaborators::default().with_token_exchanger(exchanger.clone());
    let resolver = provider(collaborators);
    let anonymous = subject("org-1", "user-1", None);
    resolver
        .invalidate_credentials(&anonymous, &token_exchange_spec())
        .await;
    assert!(exchanger.invalidations().is_empty());
    assert!(exchanger.exchanges().is_empty());
}
