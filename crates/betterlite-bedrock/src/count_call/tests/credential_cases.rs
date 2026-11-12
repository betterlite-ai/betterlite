use betterlite_models::decisions::Json;

use crate::count_call::counter::{BedrockTokenCounter, CountTokensDeployment};
use crate::count_call::handler::BedrockCountTokensHandler;

use super::logs::{Kind, capture};
use super::support::{
    FailingCredentials, Reply, Script, call, deployment, environment, fixture, header,
    user_messages,
};

const BEARER: &str = "fixture-bearer-secret";
const SECRET: &str = "fixture-secret-key";

pub(crate) async fn count_credentials() {
    static_credentials_sign().await;
    session_token_header().await;
    bearer_token_skips_lookup().await;
    environment_bearer_used().await;
    empty_api_key_shadows_environment_bearer().await;
    identities_stay_separate().await;
    missing_credentials_become_record().await;
}

fn keys(access: &str, secret: &str) -> Vec<(&'static str, Json)> {
    vec![
        ("aws_access_key_id", Json::Text(access.to_owned())),
        ("aws_secret_access_key", Json::Text(secret.to_owned())),
    ]
}

async fn counted(
    params: &CountTokensDeployment,
    built: super::support::Fixture,
) -> super::support::Sent {
    let messages = user_messages();
    let transport = built.transport.clone();
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    counter
        .count_tokens(&call(&messages, Some(params)))
        .await
        .unwrap();
    transport.request()
}

async fn static_credentials_sign() {
    let params = deployment(&keys("AKIDFIXTURE", SECRET));
    let built = fixture(Reply::ok(r#"{"inputTokens": 1}"#), environment(&[]));
    let calls = built.credentials.clone();
    let sent = counted(&params, built).await;
    let authorization = header(&sent, "authorization").unwrap();
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDFIXTURE/"));
    assert!(authorization.contains("/us-west-2/bedrock/aws4_request"));
    assert_eq!(header(&sent, "x-amz-date"), Some("20260101T000000Z"));
    assert_eq!(header(&sent, "x-amz-security-token"), None);
    assert_eq!(calls.calls(), 1);
}

async fn session_token_header() {
    let mut entries = keys("AKIDFIXTURE", SECRET);
    entries.push((
        "aws_session_token",
        Json::Text("fixture-session-token".to_owned()),
    ));
    let params = deployment(&entries);
    let built = fixture(Reply::ok(r#"{"inputTokens": 1}"#), environment(&[]));
    let sent = counted(&params, built).await;
    assert_eq!(
        header(&sent, "x-amz-security-token"),
        Some("fixture-session-token")
    );
}

async fn bearer_token_skips_lookup() {
    let params = deployment(&[("api_key", Json::Text(BEARER.to_owned()))]);
    let built = fixture(Reply::ok(r#"{"inputTokens": 2}"#), environment(&[]));
    let calls = built.credentials.clone();
    let sent = counted(&params, built).await;
    assert_eq!(
        header(&sent, "authorization"),
        Some("Bearer fixture-bearer-secret")
    );
    assert_eq!(header(&sent, "x-amz-date"), None);
    assert_eq!(calls.calls(), 0);
}

async fn environment_bearer_used() {
    let params = deployment(&[]);
    let built = fixture(
        Reply::ok(r#"{"inputTokens": 3}"#),
        environment(&[("AWS_BEARER_TOKEN_BEDROCK", BEARER)]),
    );
    let calls = built.credentials.clone();
    let sent = counted(&params, built).await;
    assert_eq!(
        header(&sent, "authorization"),
        Some("Bearer fixture-bearer-secret")
    );
    assert_eq!(calls.calls(), 0);
}

async fn empty_api_key_shadows_environment_bearer() {
    let params = deployment(&[
        ("api_key", Json::Text(String::new())),
        ("aws_access_key_id", Json::Text("AKIDFIXTURE".to_owned())),
        ("aws_secret_access_key", Json::Text(SECRET.to_owned())),
    ]);
    let built = fixture(
        Reply::ok(r#"{"inputTokens": 4}"#),
        environment(&[("AWS_BEARER_TOKEN_BEDROCK", BEARER)]),
    );
    let sent = counted(&params, built).await;
    assert!(
        header(&sent, "authorization")
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDFIXTURE/")
    );
}

async fn identities_stay_separate() {
    let alice = deployment(&keys("AKIDALICE", "alice-secret"));
    let bob = deployment(&keys("AKIDBOB", "bob-secret"));
    let alice_built = fixture(Reply::ok(r#"{"inputTokens": 5}"#), environment(&[]));
    let bob_built = fixture(Reply::ok(r#"{"inputTokens": 6}"#), environment(&[]));
    let alice_sent = counted(&alice, alice_built).await;
    let bob_sent = counted(&bob, bob_built).await;
    let alice_authorization = header(&alice_sent, "authorization").unwrap();
    let bob_authorization = header(&bob_sent, "authorization").unwrap();
    assert!(alice_authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDALICE/"));
    assert!(bob_authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDBOB/"));
    assert!(
        !alice_sent
            .headers
            .iter()
            .any(|(_, value)| value.contains("AKIDBOB"))
    );
    assert!(
        !bob_sent
            .headers
            .iter()
            .any(|(_, value)| value.contains("AKIDALICE"))
    );
}

async fn missing_credentials_become_record() {
    let messages = user_messages();
    let params = deployment(&keys("AKIDFIXTURE", SECRET));
    let (logs, guard) = capture();
    let counter = BedrockTokenCounter {
        handler: BedrockCountTokensHandler {
            transport: Script::replying(Reply::ok("{}")),
            credentials: FailingCredentials::new(),
            environment: environment(&[]),
        },
    };
    let outcome = counter
        .count_tokens(&call(&messages, Some(&params)))
        .await
        .unwrap();
    drop(guard);
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(500));
    assert!(
        outcome
            .error_message
            .as_deref()
            .unwrap()
            .starts_with("CountTokens processing error: ")
    );
    assert_eq!(logs.subjects(Kind::Warn), ["Bedrock CountTokens API error"]);
}
