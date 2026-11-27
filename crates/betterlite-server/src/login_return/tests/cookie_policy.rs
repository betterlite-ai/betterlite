use crate::login_return::redirect::ResumeRequest;
use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;

use super::support::{
    RecordingCache, forwarded_network, plain_network, public_base_url, resume, session_token,
    settings, trusted_proxy_settings,
};

const SESSION_COOKIE: &str = "token=jwt.header.payload.signature; Path=/; SameSite=lax";

async fn token_cookie(network: NetworkRequest<'_>, trusted: &TrustedNetworkSettings) -> String {
    let token = session_token();
    let user = RecordingCache::default();
    let redirect = resume(
        ResumeRequest {
            destination: Some(b"/ui/return"),
            session_token: &token,
            network,
        },
        &settings(None),
        trusted,
        &user,
    )
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
    let header = token_cookie(plain_network("http"), &TrustedNetworkSettings::default()).await;
    assert_eq!(header, SESSION_COOKIE);
}

#[tokio::test]
async fn https() {
    let header = token_cookie(plain_network("https"), &TrustedNetworkSettings::default()).await;
    assert_eq!(header, format!("{SESSION_COOKIE}; Secure"));
}

#[tokio::test]
async fn trusted_forwarding() {
    let trusted = trusted_proxy_settings();
    let forwarded = forwarded_network("10.1.2.3", "https", "http");
    let spoofed = forwarded_network("203.0.113.9", "https", "http");
    let secured = token_cookie(forwarded, &trusted).await;
    let kept = token_cookie(spoofed, &trusted).await;
    assert_eq!(secured, format!("{SESSION_COOKIE}; Secure"));
    assert_eq!(kept, SESSION_COOKIE);
}

#[tokio::test]
async fn proxy_base_url_wins() {
    let trusted = public_base_url("https://public.example");
    let header = token_cookie(plain_network("http"), &trusted).await;
    assert_eq!(header, format!("{SESSION_COOKIE}; Secure"));
}
