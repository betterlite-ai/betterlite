use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;

use super::support::{RecordingCache, resume_relative, session_token};

const SESSION_COOKIE: &str = "token=jwt.header.payload.signature; Path=/; SameSite=lax";

async fn token_cookie(network: NetworkRequest<'_>, trusted: &TrustedNetworkSettings) -> String {
    let token = session_token();
    let user = RecordingCache::default();
    let redirect = resume_relative(&token, b"/ui/return", network, trusted, &user)
        .await
        .unwrap()
        .unwrap();
    redirect
        .set_cookies
        .first()
        .cloned()
        .expect("the relative arm sets the session cookie")
}

#[tokio::test]
async fn http() {
    let network = NetworkRequest {
        scheme: "http",
        ..NetworkRequest::default()
    };
    let header = token_cookie(network, &TrustedNetworkSettings::default()).await;
    assert_eq!(header, SESSION_COOKIE);
}

#[tokio::test]
async fn https() {
    let network = NetworkRequest {
        scheme: "https",
        ..NetworkRequest::default()
    };
    let header = token_cookie(network, &TrustedNetworkSettings::default()).await;
    assert_eq!(header, format!("{SESSION_COOKIE}; Secure"));
}

#[tokio::test]
async fn trusted_forwarding() {
    let trusted = TrustedNetworkSettings {
        use_x_forwarded_for: true,
        mcp_trusted_proxy_ranges: Some(vec!["10.0.0.0/8".to_owned()]),
        ..TrustedNetworkSettings::default()
    };
    let network = NetworkRequest {
        peer: Some("10.1.2.3"),
        x_forwarded_proto: Some("https"),
        scheme: "http",
        ..NetworkRequest::default()
    };
    let header = token_cookie(network, &trusted).await;
    assert_eq!(header, format!("{SESSION_COOKIE}; Secure"));
}

#[tokio::test]
async fn untrusted_forwarding_ignored() {
    let trusted = TrustedNetworkSettings {
        use_x_forwarded_for: true,
        mcp_trusted_proxy_ranges: Some(vec!["10.0.0.0/8".to_owned()]),
        ..TrustedNetworkSettings::default()
    };
    let network = NetworkRequest {
        peer: Some("203.0.113.9"),
        x_forwarded_proto: Some("https"),
        scheme: "http",
        ..NetworkRequest::default()
    };
    let header = token_cookie(network, &trusted).await;
    assert_eq!(header, SESSION_COOKIE);
}

#[tokio::test]
async fn proxy_base_url_wins() {
    let trusted = TrustedNetworkSettings {
        proxy_base_url: "https://public.example".to_owned(),
        ..TrustedNetworkSettings::default()
    };
    let network = NetworkRequest {
        scheme: "http",
        ..NetworkRequest::default()
    };
    let header = token_cookie(network, &trusted).await;
    assert_eq!(header, format!("{SESSION_COOKIE}; Secure"));
}
