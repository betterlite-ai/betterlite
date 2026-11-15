#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_mcp::credential_contracts::auth_config::id_jag::{
    ClientAuth, ClientSecretAuth, PrivateKeyJwtAuth,
};
use betterlite_mcp::credential_contracts::errors::CredError;
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::cache::{ExchangedTokenCache, cache_ttl_seconds};
use super::post::{HttpTokenPost, PostedForm, TokenEndpointPost};
use super::{CLIENT_ASSERTION_TYPE, ExchangedToken, TokenEndpointClient};
use crate::mcp::outbound_credentials::token_exchange_provider::TokenExchangeTunables;
use crate::mcp::outbound_credentials::{RefreshFuture, WallClock};

const ENDPOINT: &str = "https://token.example/oauth/token";
const CLIENT_ID: &str = "client-1";
const NOW: i64 = 1_700_000_000;
const SIGNING_FAILED: &str = "token exchange failed: could not sign the client assertion; check client_private_key and client_assertion_signing_alg";

const RSA_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDAPlal9kzPrjXD
X39vm6fF9+CAoqqAjScmU3VVk8GAipNUKxXhd2sQpl7NwlKdIYxsyklYXT7tInxM
K3tcoyZ6xOZ9QMmklL91rfrlEpNP55OVIvpBng5xmgpCEQc9c0n76jhv/4vFKgtS
VqbIGTfk93MXcZTi6RDalwLqJ5RndVsHphoPZX8TJtm/K+myIZixVQ1d/hs+udE8
wqwyWym7bkw1Vu91tsQbtWvcLqkDAT1dC+Nl0fxOMr/0ZP045+FN2kc2lYyiniKj
2KT394N2yLx/+MmXGVrdymVVkdL8GUqJ1oeCd04+D4ADcpmaLtXfQEni4sahCRJB
hkkLCsCtAgMBAAECggEAH5tAK19gB+G9pMZIe+TXWRokrCbSVHn13qcTCpj/09k0
rr5ksGHYqu9HdURxHl9mfc5mRPkTqvDOscaRAiKFZY2GKD/6HFfOVek0ROw1vF83
qZSLbjVabOf6rsZWQ+P2sncIL2Blg5V4MLc0CtWb6xuZ2FgMC/XDQm+t2CneDCzQ
L9yZKhoFTNRfYRePaYka2H2eMXLBCXLLvoKwSl1uYSh7+iWi9c6OX13IonqQVMmr
HcqGTd68aLKFnb5zQfsPL4m1d3KrAA4MnZJFOdECLYPjvLNl2/g0ofLVi6ZuJC74
TK+QDJ90jclUHrSVwKVWAyu9xnGPpF/qAhLAVze02wKBgQD7/ax6Tke0foWsmegn
KN5q1WTnHxId343BS1WsTHXyljgAK2yHXLdwH+iJ3+eaeJNHcHwXk9yJuXFjXq1z
DPJuQVQJ79iOU7B7LYrD0rrf/lE9EdUpPhm9r4hIvnk6evIw9K0tgdmm6gi0xwE9
DD+7z1YqUF/oq56exZV5twP99wKBgQDDTVJBsgRdERRW6jN6F66JjRATxIAiw4O8
kLJACMvPqBqiYUlzxcns4AKoTNT5ZxR7eIXk8KB3dhBvr0zIQ4MoCw7KYLEccwZt
RfcTmouwZyIIzI725aylF79QbtEuUIn+VdG06oEDB02qW1EbC5HhHIxgEuOo0KrJ
GKY5astdewKBgQCjt41erQszPlx05k2JW6kdueTjsDEiDYNYySwxVD/xSx25Jn1N
eLSJeYGoSjt8BciSSoek/eDz7fK4P+Pq8sdzUKhcCGMnHUJ01QLp9TiKnP2E8tgO
ApXQwoGF2ZKDpytBGUaGYW3aXwUnBoBMfG/QAoYV2cBmN1JID1DtEjXawwKBgQCX
7wcW5I5QeIECPKBfUx0NFRh0yGGGqeNKRqbk+fLUUUqnGFg0IKK+DGcE1wC4JWDh
zRkcybWMOeT/fbiV8y+Nb0zKU/50y2csi0lzjr8tNI5dmgV+PZ7ZsGe9Avjotu8j
VVAXFnfXC094QCIQkCekoLjz5L4HDo5Oc0adVF9xEQKBgBCfRL90hRwwO39OO3H9
FbOAICm0i58R5B84kvuHO3GPRMXH2jJLbZKOutIKi/ujSKN62pGaEOocLXl0gyl0
TTklJP+NswyLej5ku0ZdLeRpLQ6BrT0rgMJYhE4Pak9N8KPr9mSqENWc6jMMr11C
jt9qIV2fBaVbYdTslNxPSHdj
-----END PRIVATE KEY-----
";

struct FakePost {
    replies: Mutex<Vec<PostedForm>>,
    forms: Mutex<Vec<Vec<(String, String)>>>,
    endpoints: Mutex<Vec<String>>,
}

impl FakePost {
    fn with_replies(replies: Vec<PostedForm>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
            forms: Mutex::new(Vec::new()),
            endpoints: Mutex::new(Vec::new()),
        })
    }

    fn form(&self) -> BTreeMap<String, String> {
        let forms = self.forms.lock().unwrap();
        forms[0].iter().cloned().collect()
    }
}

#[async_trait]
impl TokenEndpointPost for Arc<FakePost> {
    async fn post(&self, endpoint: &str, form: &[(String, SecretString)]) -> PostedForm {
        self.endpoints.lock().unwrap().push(endpoint.to_owned());
        self.forms.lock().unwrap().push(
            form.iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
        );
        self.replies
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(PostedForm::Unreachable { kind: "NoReply" })
    }
}

struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(DateTime::from_timestamp(NOW, 0).unwrap()),
        })
    }

    fn advance(&self, seconds: i64) {
        let elapsed = TimeDelta::try_seconds(seconds).unwrap();
        *self.now.lock().unwrap() += elapsed;
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }
}

fn client(replies: Vec<PostedForm>) -> (TokenEndpointClient<Arc<FakePost>>, Arc<FakePost>) {
    let post = FakePost::with_replies(replies);
    let mut client = TokenEndpointClient::new(post.clone());
    client.clock = ManualClock::new();
    client.new_jti = || "jti-fixed".to_owned();
    (client, post)
}

fn grant() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("grant_type".to_owned(), "authorization_code".to_owned()),
        ("code".to_owned(), "code-1".to_owned()),
    ])
}

fn client_secret_auth() -> ClientAuth {
    ClientAuth::ClientSecret(ClientSecretAuth {
        client_secret: SecretString::from("sekret-1".to_owned()),
    })
}

fn jwt_auth(alg: &str, key: &str, kid: Option<&str>) -> ClientAuth {
    ClientAuth::PrivateKeyJwt(PrivateKeyJwtAuth {
        private_key: SecretString::from(key.to_owned()),
        key_id: kid.map(str::to_owned),
        signing_alg: alg.to_owned(),
    })
}

fn answered(status: u16, body: &str) -> PostedForm {
    PostedForm::Answered {
        status,
        body: body.as_bytes().to_vec(),
    }
}

fn unreadable<T: DeserializeOwned>(part: &str) -> T {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
}

#[derive(serde::Deserialize)]
struct AssertionHeader {
    alg: String,
    typ: Option<String>,
    kid: Option<String>,
}

#[derive(serde::Deserialize)]
struct AssertionClaims {
    iss: String,
    sub: String,
    aud: String,
    jti: String,
    iat: i64,
    exp: i64,
}

fn read_assertion(assertion: &str) -> (AssertionHeader, AssertionClaims) {
    let mut parts = assertion.split('.');
    let header = unreadable(parts.next().unwrap());
    let claims = unreadable(parts.next().unwrap());
    assert!(!parts.next().unwrap().is_empty());
    assert!(parts.next().is_none());
    (header, claims)
}

#[tokio::test]
async fn sends_the_grant_and_client_secret_without_an_assertion() {
    let (client, post) = client(vec![answered(
        200,
        r#"{"access_token":"minted","expires_in":3600}"#,
    )]);
    let outcome = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        ExchangedToken {
            access_token: "minted".to_owned(),
            expires_in: Some(3600),
        }
    );
    let form = post.form();
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        form.get("client_secret").map(String::as_str),
        Some("sekret-1")
    );
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert!(!form.contains_key("client_assertion"));
    assert_eq!(post.endpoints.lock().unwrap()[0], ENDPOINT);
}

#[tokio::test]
async fn posts_a_signed_private_key_assertion() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("HS256", "shared-secret", Some("kid-1")),
        )
        .await
        .unwrap();
    let form = post.form();
    assert_eq!(
        form.get("client_assertion_type").map(String::as_str),
        Some(CLIENT_ASSERTION_TYPE)
    );
    let (header, claims) = read_assertion(form.get("client_assertion").unwrap());
    assert_eq!(header.alg, "HS256");
    assert_eq!(header.typ.as_deref(), Some("JWT"));
    assert_eq!(header.kid.as_deref(), Some("kid-1"));
    assert_eq!(claims.iss, CLIENT_ID);
    assert_eq!(claims.sub, CLIENT_ID);
    assert_eq!(claims.aud, ENDPOINT);
    assert_eq!(claims.jti, "jti-fixed");
    assert_eq!(claims.iat, NOW);
    assert_eq!(claims.exp, NOW + 60);
}

#[tokio::test]
async fn signs_with_the_configured_rsa_key_and_no_kid() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("RS256", RSA_KEY, None),
        )
        .await
        .unwrap();
    let (header, claims) = read_assertion(post.form().get("client_assertion").unwrap());
    assert_eq!(header.alg, "RS256");
    assert_eq!(header.kid, None);
    assert_eq!(claims.exp - claims.iat, 60);
}

#[tokio::test]
async fn rejects_an_unknown_signing_algorithm() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    let error = client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("ES512", RSA_KEY, None),
        )
        .await
        .unwrap_err();
    assert_eq!(error, CredError::Misconfigured(SIGNING_FAILED.to_owned()));
    assert!(post.forms.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rejects_a_key_that_does_not_match_the_algorithm() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    let error = client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("RS256", "not-a-pem", None),
        )
        .await
        .unwrap_err();
    assert_eq!(error, CredError::Misconfigured(SIGNING_FAILED.to_owned()));
    assert!(post.forms.lock().unwrap().is_empty());
}

#[tokio::test]
async fn client_parameters_override_the_grant() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    let mut params = grant();
    params.insert("client_id".to_owned(), "grant-client".to_owned());
    params.insert("client_secret".to_owned(), "grant-secret".to_owned());
    client
        .fetch(ENDPOINT, CLIENT_ID, &params, &client_secret_auth())
        .await
        .unwrap();
    let form = post.form();
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        form.get("client_secret").map(String::as_str),
        Some("sekret-1")
    );
    assert_eq!(form.get("code").map(String::as_str), Some("code-1"));
}

#[test]
fn debug_hides_the_access_token() {
    let token = ExchangedToken {
        access_token: "super-secret".to_owned(),
        expires_in: Some(60),
    };
    let rendered = format!("{token:?}");
    assert!(rendered.contains("access_token: \"***\""));
    assert!(!rendered.contains("super-secret"));
}

#[tokio::test]
async fn returns_a_token_without_expiry() {
    let (client, _post) = client(vec![answered(
        200,
        r#"{"access_token":"minted","expires_in":null}"#,
    )]);
    let outcome = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap();
    assert_eq!(outcome.expires_in, None);
    assert_eq!(outcome.access_token, "minted");
}

#[tokio::test]
async fn accepts_a_redirect_status_that_was_not_followed() {
    let (client, _post) = client(vec![answered(302, r#"{"access_token":"minted"}"#)]);
    let outcome = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap();
    assert_eq!(outcome.access_token, "minted");
}

#[tokio::test]
async fn maps_a_status_of_400_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(400, r#"{"error":"bad_request"}"#)]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable("token exchange failed with status 400".to_owned())
    );
}

#[tokio::test]
async fn maps_a_four_xx_status_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(401, r#"{"error":"invalid_client"}"#)]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable("token exchange failed with status 401".to_owned())
    );
}

#[tokio::test]
async fn maps_an_unreachable_endpoint_to_upstream_unavailable() {
    let (client, _post) = client(vec![PostedForm::Unreachable {
        kind: "ConnectError",
    }]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint unreachable (ConnectError)".to_owned()
        )
    );
}

#[tokio::test]
async fn maps_a_non_json_body_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(200, "not json")]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint returned a non-JSON response".to_owned()
        )
    );
}

#[tokio::test]
async fn maps_an_empty_body_to_a_non_json_response() {
    let (client, _post) = client(vec![answered(200, "")]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint returned a non-JSON response".to_owned()
        )
    );
}

#[tokio::test]
async fn maps_a_missing_access_token_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(200, r#"{"token_type":"bearer"}"#)]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint response missing access_token".to_owned()
        )
    );
}

#[tokio::test]
async fn maps_a_non_string_access_token_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(200, r#"{"access_token":17}"#)]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint response missing access_token".to_owned()
        )
    );
}

#[tokio::test]
async fn maps_a_non_object_body_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(200, "[1,2]")]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint response missing access_token".to_owned()
        )
    );
}

#[tokio::test]
async fn maps_a_text_expires_in_to_an_invalid_expiry() {
    let (client, _post) = client(vec![answered(
        200,
        r#"{"access_token":"minted","expires_in":"3600"}"#,
    )]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint response has an invalid expires_in".to_owned()
        )
    );
}

#[tokio::test]
async fn maps_a_fractional_expires_in_to_an_invalid_expiry() {
    let (client, _post) = client(vec![answered(
        200,
        r#"{"access_token":"minted","expires_in":60.5}"#,
    )]);
    let error = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange failed: token endpoint response has an invalid expires_in".to_owned()
        )
    );
}

#[tokio::test]
async fn keeps_an_empty_access_token() {
    let (client, _post) = client(vec![answered(200, r#"{"access_token":""}"#)]);
    let outcome = client
        .fetch(ENDPOINT, CLIENT_ID, &grant(), &client_secret_auth())
        .await
        .unwrap();
    assert_eq!(outcome.access_token, "");
}

#[test]
fn the_ttl_subtracts_the_buffer_and_floors_at_the_minimum() {
    let tunables = TokenExchangeTunables::default();
    assert_eq!(
        cache_ttl_seconds(Some(3600), &tunables),
        Duration::from_secs(3540)
    );
    assert_eq!(
        cache_ttl_seconds(Some(71), &tunables),
        Duration::from_secs(11)
    );
    assert_eq!(
        cache_ttl_seconds(Some(70), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(65), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(60), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(5), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(0), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(-5), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(None, &tunables),
        Duration::from_secs(3540)
    );
}

#[test]
fn the_tunables_default_to_the_litellm_constants() {
    let tunables = TokenExchangeTunables::default();
    assert_eq!(tunables.default_ttl, Duration::from_secs(3600));
    assert_eq!(tunables.expiry_buffer, Duration::from_secs(60));
    assert_eq!(tunables.min_ttl, Duration::from_secs(10));
    assert_eq!(tunables.cache_max_size, 500);
}

fn minted(
    computes: Arc<AtomicUsize>,
    token: &'static str,
    expires_in: Option<i64>,
) -> RefreshFuture<'static, ExchangedToken, CredError> {
    Box::pin(async move {
        computes.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok(ExchangedToken {
            access_token: token.to_owned(),
            expires_in,
        })
    })
}

fn cache() -> (ExchangedTokenCache, Arc<ManualClock>) {
    let clock = ManualClock::new();
    (
        ExchangedTokenCache::new(clock.clone(), TokenExchangeTunables::default()),
        clock,
    )
}

#[tokio::test]
async fn a_hit_skips_the_compute() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    let first = cache
        .get_or_compute("key", "", minted(computes.clone(), "token-a", Some(600)))
        .await
        .unwrap();
    let second = cache
        .get_or_compute("key", "", minted(computes.clone(), "token-b", Some(600)))
        .await
        .unwrap();
    assert_eq!(first, "token-a");
    assert_eq!(second, "token-a");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_fingerprint_mismatch_recomputes() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute(
            "key",
            "user-a",
            minted(computes.clone(), "token-a", Some(600)),
        )
        .await
        .unwrap();
    let other = cache
        .get_or_compute(
            "key",
            "user-b",
            minted(computes.clone(), "token-b", Some(600)),
        )
        .await
        .unwrap();
    let again = cache
        .get_or_compute(
            "key",
            "user-b",
            minted(computes.clone(), "token-c", Some(600)),
        )
        .await
        .unwrap();
    assert_eq!(other, "token-b");
    assert_eq!(again, "token-b");
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn concurrent_misses_on_one_key_share_a_compute() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    let (first, second) = tokio::join!(
        cache.get_or_compute("key", "", minted(computes.clone(), "token-a", Some(600))),
        cache.get_or_compute("key", "", minted(computes.clone(), "token-b", Some(600))),
    );
    assert_eq!(first.unwrap(), "token-a");
    assert_eq!(second.unwrap(), "token-a");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn invalidate_forces_the_next_compute() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute("key", "", minted(computes.clone(), "token-a", Some(600)))
        .await
        .unwrap();
    cache.invalidate("key");
    let second = cache
        .get_or_compute("key", "", minted(computes.clone(), "token-b", Some(600)))
        .await
        .unwrap();
    assert_eq!(second, "token-b");
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalidate_during_a_compute_wins_over_its_write() {
    let (cache, _clock) = cache();
    let (started_sender, started) = tokio::sync::oneshot::channel::<()>();
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let flight = cache.get_or_compute(
        "key",
        "",
        Box::pin(async move {
            drop(started_sender);
            drop(released.await);
            Ok(ExchangedToken {
                access_token: "stale".to_owned(),
                expires_in: Some(600),
            })
        }),
    );
    let evict = async {
        started.await.unwrap();
        cache.invalidate("key");
        drop(release.send(()));
    };
    let (outcome, ()) = tokio::join!(flight, evict);
    assert_eq!(outcome.unwrap(), "stale");
    let computes = Arc::new(AtomicUsize::new(0));
    let fresh = cache
        .get_or_compute("key", "", minted(computes.clone(), "fresh", Some(600)))
        .await
        .unwrap();
    assert_eq!(fresh, "fresh");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_compute_is_not_stored() {
    let (cache, _clock) = cache();
    let failure = cache
        .get_or_compute(
            "key",
            "",
            Box::pin(async { Err(CredError::UpstreamUnavailable("endpoint down".to_owned())) }),
        )
        .await;
    assert_eq!(
        failure,
        Err(CredError::UpstreamUnavailable("endpoint down".to_owned()))
    );
    let computes = Arc::new(AtomicUsize::new(0));
    let second = cache
        .get_or_compute("key", "", minted(computes.clone(), "token-a", Some(600)))
        .await
        .unwrap();
    let third = cache
        .get_or_compute("key", "", minted(computes.clone(), "token-b", Some(600)))
        .await
        .unwrap();
    assert_eq!(second, "token-a");
    assert_eq!(third, "token-a");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_entry_expires_at_the_ttl_after_the_buffer() {
    let (cache, clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute("key", "", minted(computes.clone(), "token-a", Some(65)))
        .await
        .unwrap();
    clock.advance(9);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(computes.clone(), "unused", None))
            .await
            .unwrap(),
        "token-a"
    );
    clock.advance(1);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(computes.clone(), "token-b", None))
            .await
            .unwrap(),
        "token-b"
    );
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_missing_expiry_uses_the_default_ttl() {
    let (cache, clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute("key", "", minted(computes.clone(), "token-a", None))
        .await
        .unwrap();
    clock.advance(3539);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(computes.clone(), "token-b", None))
            .await
            .unwrap(),
        "token-a"
    );
    clock.advance(1);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(computes.clone(), "token-c", None))
            .await
            .unwrap(),
        "token-c"
    );
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_cache_evicts_at_capacity() {
    let clock = ManualClock::new();
    let tunables = TokenExchangeTunables {
        cache_max_size: 1,
        default_ttl: Duration::from_secs(3600),
        min_ttl: Duration::from_secs(10),
        expiry_buffer: Duration::from_secs(60),
    };
    let cache = ExchangedTokenCache::new(clock, tunables);
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute("first", "", minted(computes.clone(), "token-a", Some(600)))
        .await
        .unwrap();
    cache
        .get_or_compute("second", "", minted(computes.clone(), "token-b", Some(600)))
        .await
        .unwrap();
    let first = cache
        .get_or_compute("first", "", minted(computes.clone(), "token-c", Some(600)))
        .await
        .unwrap();
    assert_eq!(first, "token-c");
    assert_eq!(computes.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn the_cache_evicts_the_entry_that_expires_soonest() {
    let clock = ManualClock::new();
    let tunables = TokenExchangeTunables {
        cache_max_size: 2,
        default_ttl: Duration::from_secs(3600),
        min_ttl: Duration::from_secs(10),
        expiry_buffer: Duration::from_secs(60),
    };
    let cache = ExchangedTokenCache::new(clock, tunables);
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute("long", "", minted(computes.clone(), "token-a", Some(3600)))
        .await
        .unwrap();
    cache
        .get_or_compute("short", "", minted(computes.clone(), "token-b", Some(65)))
        .await
        .unwrap();
    cache
        .get_or_compute("third", "", minted(computes.clone(), "token-c", Some(3600)))
        .await
        .unwrap();
    let long = cache
        .get_or_compute("long", "", minted(computes.clone(), "unused", None))
        .await
        .unwrap();
    let short = cache
        .get_or_compute("short", "", minted(computes.clone(), "token-d", Some(3600)))
        .await
        .unwrap();
    assert_eq!(long, "token-a");
    assert_eq!(short, "token-d");
    assert_eq!(computes.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn a_zero_max_size_caches_nothing() {
    let clock = ManualClock::new();
    let tunables = TokenExchangeTunables {
        cache_max_size: 0,
        default_ttl: Duration::from_secs(3600),
        min_ttl: Duration::from_secs(10),
        expiry_buffer: Duration::from_secs(60),
    };
    let cache = ExchangedTokenCache::new(clock, tunables);
    let computes = Arc::new(AtomicUsize::new(0));
    let first = cache
        .get_or_compute("key", "", minted(computes.clone(), "token-a", Some(600)))
        .await
        .unwrap();
    let second = cache
        .get_or_compute("key", "", minted(computes.clone(), "token-b", Some(600)))
        .await
        .unwrap();
    assert_eq!(first, "token-a");
    assert_eq!(second, "token-b");
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn form() -> Vec<(String, SecretString)> {
    vec![
        ("grant_type".to_owned(), secret("authorization_code")),
        ("client_id".to_owned(), secret("client-1")),
        ("client_secret".to_owned(), secret("s e/c")),
    ]
}

fn poster(limits: TransportLimits) -> HttpTokenPost {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    HttpTokenPost {
        client: Arc::new(ProxiedClient::build(base_builder(), &policy, limits).unwrap()),
        deadline: Duration::from_secs(5),
    }
}

fn response(status_line: &str, body: &str) -> String {
    format!(
        "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !complete(&collected) {
        let read = stream.read(&mut buffer).await.expect("the request arrives");
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&buffer[..read]);
    }
    collected
}

fn complete(request: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(request) else {
        return false;
    };
    let Some(split) = text.find("\r\n\r\n") else {
        return false;
    };
    let Some(head) = text.get(..split) else {
        return false;
    };
    let declared = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    declared.is_none_or(|length| {
        split
            .checked_add(4)
            .and_then(|start| start.checked_add(length))
            .is_some_and(|total| text.len() >= total)
    })
}

async fn answer(listener: TcpListener, reply: String) -> Vec<u8> {
    let (mut stream, _peer) = listener.accept().await.expect("the client connects");
    let request = read_request(&mut stream).await;
    stream
        .write_all(reply.as_bytes())
        .await
        .expect("the response is written");
    stream.flush().await.expect("the response is flushed");
    request
}

#[tokio::test]
async fn posts_the_form_urlencoded_body_to_the_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(answer(
        listener,
        response("HTTP/1.1 200 OK", r#"{"access_token":"minted"}"#),
    ));
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form())
        .await;
    let request = String::from_utf8(tasks.join_next().await.unwrap().unwrap()).unwrap();
    assert!(request.starts_with("POST /token HTTP/1.1\r\n"));
    assert!(request.contains("content-type: application/x-www-form-urlencoded\r\n"));
    assert!(
        request.ends_with("grant_type=authorization_code&client_id=client-1&client_secret=s+e%2Fc")
    );
    let PostedForm::Answered { status, body } = outcome else {
        panic!("the endpoint answered");
    };
    assert_eq!(status, 200);
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        r#"{"access_token":"minted"}"#
    );
}

#[tokio::test]
async fn reports_a_connection_failure_as_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form())
        .await;
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the endpoint is closed");
    };
    assert_eq!(kind, "TransportError");
}

#[tokio::test]
async fn reports_a_body_past_the_limit_as_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(answer(
        listener,
        response("HTTP/1.1 200 OK", &"x".repeat(64)),
    ));
    let limits = TransportLimits {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(5),
        body_bytes: 16,
        header_bytes: 4096,
    };
    let outcome = poster(limits)
        .post(&format!("http://{address}/token"), &form())
        .await;
    drop(tasks.join_next().await);
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the body is past the limit");
    };
    assert_eq!(kind, "TransportError");
}

#[tokio::test]
async fn reports_a_url_that_cannot_be_parsed() {
    let outcome = poster(TransportLimits::DEFAULT)
        .post("not a url", &form())
        .await;
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the url cannot be parsed");
    };
    assert_eq!(kind, "InvalidUrl");
}

#[tokio::test]
async fn a_dropped_connection_is_attempted_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut stream, _peer) = listener.accept().await.expect("the client connects");
        drop(read_request(&mut stream).await);
        drop(stream);
        let retried = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(retried.is_err(), "the failure is not retried");
    });
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form())
        .await;
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the endpoint dropped the connection");
    };
    assert_eq!(kind, "TransportError");
    tasks.join_next().await.unwrap().unwrap();
}
