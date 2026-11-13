use std::fs;
use std::path::Path;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{
    Clock, Reply, Script, SentRequest, answer, environment, fixture, secret, token_reply,
};
use crate::watsonx_auth::error::WatsonxError;
use crate::watsonx_auth::iam::{DEFAULT_IAM_URL, decode_iam_token_response, get_watsonx_iam_url};

#[tokio::test]
async fn iam_token_is_fetched_once_per_key() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(DEFAULT_IAM_URL, vec![token_reply("iam-token", 3600)]);
    let client = fixture(script, &clock, &[]);
    let key = secret("key-1");
    let first = client.generate_iam_token(Some(&key)).await.unwrap();
    let second = client.generate_iam_token(Some(&key)).await.unwrap();
    assert_eq!(first.expose_secret(), "iam-token");
    assert_eq!(second.expose_secret(), "iam-token");
    assert_eq!(client.transport.take().len(), 1);
}

#[tokio::test]
async fn iam_token_is_cached_until_expires_in_minus_ten_seconds() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(
        DEFAULT_IAM_URL,
        vec![token_reply("first", 20), token_reply("second", 20)],
    );
    let client = fixture(script, &clock, &[]);
    let key = secret("key-1");
    let first = client.generate_iam_token(Some(&key)).await.unwrap();
    assert_eq!(first.expose_secret(), "first");
    clock.advance(10.0_f64);
    let at_boundary = client.generate_iam_token(Some(&key)).await.unwrap();
    assert_eq!(at_boundary.expose_secret(), "first");
    clock.advance(1.0_f64);
    let expired = client.generate_iam_token(Some(&key)).await.unwrap();
    assert_eq!(expired.expose_secret(), "second");
    assert_eq!(client.transport.take().len(), 2);
}

#[tokio::test]
async fn iam_token_expires_at_once_when_expires_in_is_below_the_buffer() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(
        DEFAULT_IAM_URL,
        vec![token_reply("first", 5), token_reply("second", 5)],
    );
    let client = fixture(script, &clock, &[]);
    let key = secret("key-1");
    let first = client.generate_iam_token(Some(&key)).await.unwrap();
    assert_eq!(first.expose_secret(), "first");
    clock.advance(1.0_f64);
    let second = client.generate_iam_token(Some(&key)).await.unwrap();
    assert_eq!(second.expose_secret(), "second");
    assert_eq!(client.transport.take().len(), 2);
}

#[tokio::test]
async fn iam_token_without_a_key_argument_never_hits_the_cache() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(
        DEFAULT_IAM_URL,
        vec![token_reply("first", 3600), token_reply("second", 3600)],
    );
    let client = fixture(script, &clock, &[("WX_API_KEY", "environment-key")]);
    let first = client.generate_iam_token(None).await.unwrap();
    let second = client.generate_iam_token(None).await.unwrap();
    assert_eq!(first.expose_secret(), "first");
    assert_eq!(second.expose_secret(), "second");
    assert_eq!(client.transport.take().len(), 2);
}

#[tokio::test]
async fn iam_token_without_any_api_key_is_an_error() {
    let clock = Clock::new();
    let client = fixture(Script::default(), &clock, &[]);
    let error = client.generate_iam_token(None).await.unwrap_err();
    assert!(matches!(error, WatsonxError::IamApiKeyRequired));
    assert!(client.transport.take().is_empty());
}

#[tokio::test]
async fn empty_iam_api_key_argument_is_sent_as_given() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(DEFAULT_IAM_URL, vec![token_reply("iam-token", 3600)]);
    let client = fixture(script, &clock, &[("WX_API_KEY", "environment-key")]);
    let empty = secret("");
    let token = client.generate_iam_token(Some(&empty)).await.unwrap();
    assert_eq!(token.expose_secret(), "iam-token");
    let requests = client.transport.take();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.ends_with("&apikey="));
}

#[rstest]
#[case::bad_request(400)]
#[case::server_error(500)]
#[case::redirect(302)]
#[tokio::test]
async fn iam_status_outside_success_is_an_error(#[case] status: u16) {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(DEFAULT_IAM_URL, vec![answer(status, "{}")]);
    let client = fixture(script, &clock, &[]);
    let key = secret("key-1");
    let error = client.generate_iam_token(Some(&key)).await.unwrap_err();
    assert!(matches!(
        error,
        WatsonxError::IamStatus { status: found, .. } if found == status
    ));
    assert_eq!(client.transport.take().len(), 1);
}

#[rstest]
#[case::empty_object("{}")]
#[case::without_access_token(r#"{"expires_in":3600}"#)]
#[case::without_expires_in(r#"{"access_token":"token"}"#)]
#[case::not_json("not json")]
#[case::array("[]")]
#[case::non_string_token(r#"{"access_token":5,"expires_in":3600}"#)]
#[case::non_numeric_expiry(r#"{"access_token":"token","expires_in":"soon"}"#)]
#[tokio::test]
async fn iam_reply_without_access_token_or_expires_in_is_an_error(#[case] body: &str) {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(DEFAULT_IAM_URL, vec![answer(200, body)]);
    let client = fixture(script, &clock, &[]);
    let key = secret("key-1");
    let error = client.generate_iam_token(Some(&key)).await.unwrap_err();
    assert!(matches!(
        error,
        WatsonxError::IamReply(_) | WatsonxError::IamReplyField { .. }
    ));
}

#[tokio::test]
async fn iam_reply_ignores_unknown_fields() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(
        DEFAULT_IAM_URL,
        vec![answer(
            200,
            r#"{"access_token":"token","expires_in":3600,"token_type":"Bearer"}"#,
        )],
    );
    let client = fixture(script, &clock, &[]);
    let key = secret("key-1");
    let token = client.generate_iam_token(Some(&key)).await.unwrap();
    assert_eq!(token.expose_secret(), "token");
}

#[tokio::test]
async fn iam_transport_failure_is_an_error() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(DEFAULT_IAM_URL, vec![Reply::Failure]);
    let client = fixture(script, &clock, &[]);
    let key = secret("key-1");
    let error = client.generate_iam_token(Some(&key)).await.unwrap_err();
    assert!(matches!(error, WatsonxError::IamTransport(_)));
}

#[tokio::test]
async fn iam_request_carries_the_form_body_and_json_headers() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(
        "https://iam.example/identity/token",
        vec![token_reply("iam-token", 3600)],
    );
    let client = fixture(
        script,
        &clock,
        &[("WATSONX_IAM_URL", "https://iam.example/identity/token")],
    );
    let key = secret("k+&=");
    client.generate_iam_token(Some(&key)).await.unwrap();
    let requests = client.transport.take();
    assert_eq!(
        requests[0],
        SentRequest {
            method: "POST".to_owned(),
            url: "https://iam.example/identity/token".to_owned(),
            content_type: Some("application/x-www-form-urlencoded".to_owned()),
            accept: Some("application/json".to_owned()),
            body: "grant_type=urn%3Aibm%3Aparams%3Aoauth%3Agrant-type%3Aapikey&apikey=k%2B%26%3D"
                .to_owned(),
        }
    );
}

#[rstest]
#[case::fallback(&[], DEFAULT_IAM_URL)]
#[case::environment(&[("WATSONX_IAM_URL", "https://iam.example/token")], "https://iam.example/token")]
#[case::empty_environment(&[("WATSONX_IAM_URL", "")], DEFAULT_IAM_URL)]
fn iam_url_comes_from_the_environment_or_the_default(
    #[case] values: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(get_watsonx_iam_url(&environment(values)), expected);
}

#[tokio::test]
async fn generate_watsonx_token_returns_the_given_token() {
    let clock = Clock::new();
    let client = fixture(Script::default(), &clock, &[]);
    let given = secret("given-token");
    let from_given = client
        .generate_watsonx_token(None, Some(&given))
        .await
        .unwrap();
    assert_eq!(from_given.expose_secret(), "given-token");
    let empty = secret("");
    let from_empty = client
        .generate_watsonx_token(None, Some(&empty))
        .await
        .unwrap();
    assert_eq!(from_empty.expose_secret(), "");
    assert!(client.transport.take().is_empty());
}

#[test]
fn replays_the_watsonx_iam_token_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/watsonx_iam_token");
    let entries = fs::read_dir(corpus)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(entries.len() >= 4);
    assert!(entries.into_iter().all(|entry| {
        let bytes = fs::read(entry.path()).unwrap();
        let outcome = decode_iam_token_response(&bytes);
        match entry.file_name().to_str().unwrap() {
            "valid" => {
                let reply = outcome.unwrap();
                reply.access_token.expose_secret() == "token-1"
                    && reply.expires_in.to_bits() == 3600.0_f64.to_bits()
            }
            _ => outcome.is_err(),
        }
    }));
}
