use super::fixture::{Probe, Reply, fixture_probe, now, run, secret};
use super::support::titan_body;
use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::transport::EmbeddingRequest;
use crate::embedding_call::error::EmbeddingCallError;
use crate::embedding_call::sign::{SigningInput, sign_item};

const ACCESS: &str = "AKIAREDACTIONSAMPLE0";
const SIGNING_SECRET: &str = "redaction-secret-material";
const SESSION: &str = "redaction-session-token";

#[test]
pub(super) fn embedding_call_secret_safe_failure() {
    failure_paths_keep_their_credentials_out_of_debug();
    a_signed_request_names_its_headers_only();
    a_signing_failure_carries_no_credentials();
    a_successful_reply_carries_no_credentials();
}

fn failure_paths_keep_their_credentials_out_of_debug() {
    for outcome in failure_runs() {
        assert_secret_safe(&format!("{:?}", outcome.outcome));
        assert_secret_safe(&format!("{:?}", outcome.details()));
    }
}

fn a_signed_request_names_its_headers_only() {
    let credentials = Credentials {
        access_key_id: secret(ACCESS),
        secret_access_key: secret(SIGNING_SECRET),
        session_token: Some(secret(SESSION)),
    };
    let signed = sign_item(
        b"{\"inputText\":\"hello\"}",
        SigningInput {
            endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.titan-embed-text-v1/invoke",
            credentials: Some(&credentials),
            region: "us-east-1",
            extra_headers: &[],
            api_key: None,
            configured_bearer: None,
            now: now(),
        },
    )
    .expect("the fixture request signs");
    let shown = format!("{signed:?}");
    assert!(shown.contains("Authorization"));
    assert_secret_safe(&shown);
    let request = EmbeddingRequest {
        url: signed.url.clone(),
        headers: signed.headers.clone(),
        body: signed.body.clone(),
    };
    let shown = format!("{request:?}");
    assert!(shown.contains("Authorization"));
    assert_secret_safe(&shown);
}

fn a_signing_failure_carries_no_credentials() {
    let failure = sign_item(
        b"{}",
        SigningInput {
            endpoint_url: "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.titan-embed-text-v1/invoke",
            credentials: None,
            region: "us-east-1",
            extra_headers: &[],
            api_key: None,
            configured_bearer: None,
            now: now(),
        },
    )
    .expect_err("signing without credentials fails");
    let error = EmbeddingCallError::Signing(failure);
    assert_secret_safe(&error.to_string());
    assert_secret_safe(&format!("{error:?}"));
}

fn a_successful_reply_carries_no_credentials() {
    let outcome = run(&probe(vec![Reply::ok(&titan_body("[1.0]", 1))]));
    assert_secret_safe(&format!("{:?}", outcome.response()));
}

fn failure_runs() -> Vec<super::fixture::Run> {
    let mut cohere = probe(vec![Reply::status(400, &[], "bad request")]);
    cohere.model = String::from("cohere.embed-english-v3");
    vec![
        run(&probe(vec![Reply::status(500, &[], "boom")])),
        run(&probe(vec![Reply::Timeout])),
        run(&probe(vec![Reply::Failed])),
        run(&probe(vec![Reply::ok("[]")])),
        run(&cohere),
    ]
}

fn probe(replies: Vec<Reply>) -> Probe {
    let mut built = fixture_probe();
    built.replies = replies;
    built.access_key = String::from(ACCESS);
    built.secret_key = String::from(SIGNING_SECRET);
    built.session_token = Some(String::from(SESSION));
    built
}

fn assert_secret_safe(shown: &str) {
    for hidden in [ACCESS, SIGNING_SECRET, SESSION] {
        assert!(
            !shown.contains(hidden),
            "a debug or error value exposes a fixture credential: {shown}"
        );
    }
}
