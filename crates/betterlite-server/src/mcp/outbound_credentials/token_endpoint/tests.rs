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

mod cache;
mod http;
mod keys;

const ENDPOINT: &str = "https://token.example/oauth/token";
const CLIENT_ID: &str = "client-1";
const NOW: i64 = 1_700_000_000;
const SIGNING_FAILED: &str = "token exchange failed: could not sign the client assertion; check client_private_key and client_assertion_signing_alg";

const RSA_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDe9+NL4PTL2Kbj
xmHLDgquf7uAVwEVSrqJqd2PR5RJrPmaR8qXlw4l+1FFtekhI6YWdLiv/nGzwO17
dBkTxJy+9JBsOHny+chNU7npRQKlm4QuRmMzEb8nCrdr9uH6BF9rec51EFF7GgnI
fgN6HAgg4rRP7W7HmuV4VcBd5Gs/BRLJZpObbhVaOVBaIUZlxyP3L9exGbbKLFur
mgbRjrusO84+zDAH1w3wj7Z774RHyS4l978rnpcOt/3I5S2UNl0Q1Cy5mtPQLJaT
wWrEPSnC+peH2LA9YlcmBpdt5J3l6tNQ3CYR0KCEKaePK1zxaMfLenoYU6q1xXs/
I67i/3I9AgMBAAECggEAB0VddQKloPi6vbThFZ83c6gB/CYTyJ2EMAvCyDkZybET
GHgGXOnY+4QysVU32fP0AG/3SAzCKVrQs753uEh1VX6bfHf3NbH12VwYjpX74RAm
+ZMjvLOm5uKt1BHvN3ki7nONd/PobR006MP/oFQF/NW/v3Dn9HWyTXmh/6Q8muFJ
637cGzae2w1aHt8kMPPu5OIkHwdGNH7n//jSN//5Bh9dMoHwBnretcP37d7TPiIJ
PztKcnSjQyr8pC6+wzuc1Jq13WJn+Tc4sexyuif/LYP5/JBTLWSYwkYHemBYaQbZ
ea2dgLoTeDhazJDIvpPilIyJ/V/bENmk3NUGlaiUAQKBgQD74zPJWvZdLm8Zg2ga
DMqj39D8MF19jC/1nOkh12azyegn9EH+66KdKsBB2lGKvMbSibCY1gvpFMuxz2gN
grqRQRf4azNahSSIkyeRs/I1omRYbQzXy/04caIy8+MhsxtkD95kE5CiugC4/QQx
NIaJrWlXleyWRJpyNgQ+JXwEAQKBgQDim9BcprCGh3YCFRICwLGg8McBKsogcCgl
GaekYVW1Tez6mABEQwWfnFM+lFB6VGIrvGlyaqUkdKM4bFdGB/CLLdcEHHHvppWT
AEEg55Mjbd9C+ZQM5Rxwk2SDF9g4eBAvOyinwtOp4Vj1Ky5mEnFyC+SkSiJFJvc2
54lLAnp+PQKBgBONNDTZ8auT8h253HfBLG2B7vfUTT10hzh6YII29QrZjkKjrAe8
hJfMfLXf21p4grpO471JSA1khtcbcUkzL4Yc1l56+7Z4BTjwqy6dGBzfqQhQFqId
fF9IOr3XvnacL3EZ5jpAZoarB2lUgmHLYPII2tdolL+pPY/F75hfUnwBAoGBAL8Z
76ly2UNaUWaIJlwz7Jhw9SJC15prey9uZzX077aqNMNQFEl8y+1Cg+Euan//UZrY
lYRr80C/LDaVBzSD1ZNJ4R4SbvNPZKxHdqUtJaSnL7TnjLcjL8ywP/9dyHY2qk9q
cGqpnp6Q79gxxjtGKm2nIfZ6nlP6/FhNAjdDq9FxAoGBAOEuaHfXOhVl8ZySu/bT
Rz6ipz3PkrNa9Z+MQpcGIIjg/v1m+J0hhQLcfmyyhxbNKEfUiqORTIvI5NuJSiAv
/SxOXdO8VY9NdxzwyYbw/e33DwI0ZAFBY6ecQKFLRbobnRM+S0rA6IllSorj2eVj
dLmbwV/RiHkRQXNe1Tbuldnp
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

fn unreadable<T>(part: &str) -> T
where
    T: DeserializeOwned,
{
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
async fn maps_a_sequence_body_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(200, r#"["minted"]"#)]);
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
async fn maps_a_sequence_body_with_an_expiry_to_upstream_unavailable() {
    let (client, _post) = client(vec![answered(200, r#"["minted", 3600]"#)]);
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
