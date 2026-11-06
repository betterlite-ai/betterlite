#![cfg(test)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use url::form_urlencoded;

use super::{
    BearerTokens, CredentialsError, TokenClock, TokenHttpResponse, TokenRequest, TokenTransport,
};
use crate::watsonx_credentials::params::WxoRequestParams;
use crate::watsonx_credentials::token_cache::{
    BoundedTokenCache, TokenStore as _, token_cache_key,
};

const WALL: i64 = 1_700_000_000;
const HOST: &str = "https://cp4d.example///";

struct Clock {
    monotonic: Mutex<u64>,
}

impl TokenClock for Clock {
    fn monotonic(&self) -> Duration {
        Duration::from_secs(*self.monotonic.lock().unwrap())
    }
    fn unix_seconds(&self) -> i64 {
        WALL
    }
}

struct Transport {
    answers: Mutex<VecDeque<(u16, String)>>,
    sent: Mutex<Vec<(String, String, String)>>,
}

impl TokenTransport for &Transport {
    type Error = Infallible;

    fn post(
        &self,
        request: &TokenRequest,
    ) -> impl Future<Output = Result<TokenHttpResponse, Infallible>> + Send {
        self.sent.lock().unwrap().push((
            request.url.clone(),
            request.content_type.to_owned(),
            String::from_utf8(request.body.expose_secret().to_vec()).unwrap(),
        ));
        let (status, body) = self.answers.lock().unwrap().pop_front().unwrap();
        ready(Ok(TokenHttpResponse {
            status,
            body: SecretSlice::from(body.into_bytes()),
        }))
    }
}

fn transport(answers: &[(u16, &str)]) -> Transport {
    Transport {
        answers: Mutex::new(
            answers
                .iter()
                .map(|(status, body)| (*status, (*body).to_owned()))
                .collect(),
        ),
        sent: Mutex::new(Vec::new()),
    }
}

fn tokens(transport: &Transport, at: u64) -> BearerTokens<&Transport, Clock, BoundedTokenCache> {
    BearerTokens {
        transport,
        clock: Clock {
            monotonic: Mutex::new(at),
        },
        store: BoundedTokenCache::new(NonZeroUsize::new(8).unwrap()),
    }
}

fn set_time(tokens: &BearerTokens<&Transport, Clock, BoundedTokenCache>, at: u64) {
    *tokens.clock.monotonic.lock().unwrap() = at;
}

fn requests_sent(transport: &Transport) -> usize {
    transport.sent.lock().unwrap().len()
}

fn params(auth_mode: &str, username: Option<&str>) -> WxoRequestParams {
    WxoRequestParams {
        cp4d_host: HOST.to_owned(),
        instance_id: "inst".to_owned(),
        wxo_agent_id: "agent".to_owned(),
        api_key: SecretString::from("key&=1"),
        username: username.map(str::to_owned),
        auth_mode: auth_mode.to_owned(),
        thread_id: None,
    }
}

#[tokio::test]
async fn cp4d_posts_json_to_the_authorize_endpoint_without_trailing_slashes() {
    let transport = transport(&[(200, r#"{"token": "tok", "expiration": 1700001000}"#)]);
    let tokens = tokens(&transport, 10);
    let token = tokens
        .bearer_token(&params("cp4d", Some("bob")))
        .await
        .unwrap();
    assert_eq!(token.expose_secret(), "tok");
    let sent = transport.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "https://cp4d.example/icp4d-api/v1/authorize");
    assert_eq!(sent[0].1, "application/json");
    assert_eq!(sent[0].2, r#"{"username":"bob","api_key":"key&=1"}"#);
}

#[tokio::test]
async fn ibm_cloud_posts_the_apikey_grant_form_to_the_iam_endpoint() {
    let transport = transport(&[(200, r#"{"access_token": "tok", "expires_in": 600}"#)]);
    let tokens = tokens(&transport, 10);
    let token = tokens
        .bearer_token(&params("ibm_cloud", None))
        .await
        .unwrap();
    assert_eq!(token.expose_secret(), "tok");
    let sent = transport.sent.lock().unwrap().clone();
    assert_eq!(sent[0].0, "https://iam.cloud.ibm.com/identity/token");
    assert_eq!(sent[0].1, "application/x-www-form-urlencoded");
    let pairs = form_urlencoded::parse(sent[0].2.as_bytes())
        .into_owned()
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        [
            (
                "grant_type".to_owned(),
                "urn:ibm:params:oauth:grant-type:apikey".to_owned()
            ),
            ("apikey".to_owned(), "key&=1".to_owned()),
        ]
    );
}

#[tokio::test]
async fn unknown_auth_mode_takes_the_cp4d_path_like_litellm() {
    let transport = transport(&[(200, r#"{"token": "tok"}"#)]);
    let tokens = tokens(&transport, 10);
    tokens
        .bearer_token(&params("other", Some("bob")))
        .await
        .unwrap();
    assert!(
        transport.sent.lock().unwrap()[0]
            .0
            .ends_with("/icp4d-api/v1/authorize")
    );
}

#[tokio::test]
async fn cp4d_without_username_fails_before_any_request() {
    let transport = transport(&[]);
    let tokens = tokens(&transport, 10);
    let error = tokens
        .bearer_token(&params("cp4d", None))
        .await
        .unwrap_err();
    assert!(matches!(error, CredentialsError::MissingUsername));
    assert_eq!(requests_sent(&transport), 0);
}

#[rstest]
#[case::server_error(500)]
#[case::unauthorized(401)]
#[case::redirect(302)]
#[tokio::test]
async fn non_success_status_is_an_error_and_nothing_is_cached(#[case] status: u16) {
    let transport = transport(&[
        (status, r#"{"token": "tok"}"#),
        (200, r#"{"token": "tok"}"#),
    ]);
    let tokens = tokens(&transport, 10);
    let error = tokens
        .bearer_token(&params("cp4d", Some("bob")))
        .await
        .unwrap_err();
    assert!(matches!(error, CredentialsError::Status { status: seen } if seen == status));
    tokens
        .bearer_token(&params("cp4d", Some("bob")))
        .await
        .unwrap();
    assert_eq!(requests_sent(&transport), 2);
}

#[tokio::test]
async fn undecodable_answer_is_a_decode_error() {
    let transport = transport(&[(200, "not json")]);
    let tokens = tokens(&transport, 10);
    let error = tokens
        .bearer_token(&params("cp4d", Some("bob")))
        .await
        .unwrap_err();
    assert!(matches!(error, CredentialsError::Decode(_)));
}

#[tokio::test]
async fn cached_token_is_reused_until_expiry_minus_sixty_seconds() {
    let transport = transport(&[
        (200, r#"{"access_token": "first", "expires_in": 160}"#),
        (200, r#"{"access_token": "second", "expires_in": 160}"#),
    ]);
    let tokens = tokens(&transport, 1000);
    let params = params("ibm_cloud", None);
    let first = tokens.bearer_token(&params).await.unwrap();
    assert_eq!(first.expose_secret(), "first");
    set_time(&tokens, 1099);
    let reused = tokens.bearer_token(&params).await.unwrap();
    assert_eq!(reused.expose_secret(), "first");
    assert_eq!(requests_sent(&transport), 1);
    set_time(&tokens, 1100);
    let refreshed = tokens.bearer_token(&params).await.unwrap();
    assert_eq!(refreshed.expose_secret(), "second");
    assert_eq!(requests_sent(&transport), 2);
}

#[rstest]
#[case::cp4d_absolute_epoch("cp4d", r#"{"token": "t", "expiration": 1700000160}"#)]
#[case::ibm_cloud_duration("ibm_cloud", r#"{"access_token": "t", "expires_in": 160}"#)]
#[tokio::test]
async fn cached_expiry_subtracts_sixty_seconds_in_both_modes(
    #[case] mode: &str,
    #[case] body: &str,
) {
    let transport = transport(&[(200, body), (200, body)]);
    let tokens = tokens(&transport, 500);
    let params = params(mode, Some("bob"));
    tokens.bearer_token(&params).await.unwrap();
    set_time(&tokens, 599);
    tokens.bearer_token(&params).await.unwrap();
    assert_eq!(requests_sent(&transport), 1);
    set_time(&tokens, 600);
    tokens.bearer_token(&params).await.unwrap();
    assert_eq!(requests_sent(&transport), 2);
}

#[rstest]
#[case::exactly_the_buffer(60)]
#[case::inside_the_buffer(30)]
#[case::already_expired(0)]
#[tokio::test]
async fn token_with_no_life_after_the_buffer_is_not_cached(#[case] expires_in: u32) {
    let body = format!(r#"{{"access_token": "t", "expires_in": {expires_in}}}"#);
    let transport = transport(&[(200, &body), (200, &body)]);
    let tokens = tokens(&transport, 500);
    let params = params("ibm_cloud", None);
    tokens.bearer_token(&params).await.unwrap();
    tokens.bearer_token(&params).await.unwrap();
    assert_eq!(requests_sent(&transport), 2);
}

#[tokio::test]
async fn cached_tokens_do_not_cross_modes_or_users() {
    let transport = transport(&[
        (200, r#"{"token": "a"}"#),
        (200, r#"{"access_token": "b"}"#),
        (200, r#"{"token": "c"}"#),
    ]);
    let tokens = tokens(&transport, 10);
    let bob = params("cp4d", Some("bob"));
    tokens.bearer_token(&bob).await.unwrap();
    tokens
        .bearer_token(&params("ibm_cloud", Some("bob")))
        .await
        .unwrap();
    tokens
        .bearer_token(&params("cp4d", Some("alice")))
        .await
        .unwrap();
    assert_eq!(requests_sent(&transport), 3);
    tokens.bearer_token(&bob).await.unwrap();
    assert_eq!(requests_sent(&transport), 3);
}

#[tokio::test]
async fn expired_cache_entries_are_evicted_on_every_fetch() {
    let transport = transport(&[
        (200, r#"{"access_token": "a", "expires_in": 100}"#),
        (200, r#"{"token": "b"}"#),
        (200, r#"{"token": "c"}"#),
    ]);
    let tokens = tokens(&transport, 0);
    tokens
        .bearer_token(&params("ibm_cloud", None))
        .await
        .unwrap();
    let key = token_cache_key("ibm_cloud", HOST, None, &SecretString::from("key&=1"));
    assert!(tokens.store.lookup(&key, Duration::from_secs(39)).is_some());
    set_time(&tokens, 500);
    tokens
        .bearer_token(&params("cp4d", Some("bob")))
        .await
        .unwrap();
    assert!(tokens.store.lookup(&key, Duration::ZERO).is_none());
}
