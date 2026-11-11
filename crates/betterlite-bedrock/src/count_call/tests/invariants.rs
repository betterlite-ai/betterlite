use betterlite_models::decisions::Json;

use crate::count_call::counter::{BedrockTokenCounter, CountTokensDeployment, TokenCountResponse};

use super::logs::capture;
use super::support::{
    Fixture, Reply, call, deployment, environment, fixture, header, user_messages,
};

const ALICE_KEY: &str = "AKIDALICE";
const BOB_KEY: &str = "AKIDBOB";

fn keys(access: &str, secret: &str) -> Vec<(&'static str, Json)> {
    vec![
        ("aws_access_key_id", Json::Text(access.to_owned())),
        ("aws_secret_access_key", Json::Text(secret.to_owned())),
    ]
}

async fn counted(built: Fixture, params: &CountTokensDeployment) -> TokenCountResponse {
    let messages = user_messages();
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    counter
        .count_tokens(&call(&messages, Some(params)))
        .await
        .unwrap()
}

#[tokio::test]
async fn count_call_credential_identity_isolation() {
    let alice = deployment(&keys(ALICE_KEY, "alice-secret"));
    let bob = deployment(&keys(BOB_KEY, "bob-secret"));
    let alice_built = fixture(Reply::ok(r#"{"inputTokens": 1}"#), environment(&[]));
    let bob_built = fixture(Reply::ok(r#"{"inputTokens": 2}"#), environment(&[]));
    let alice_transport = alice_built.transport.clone();
    let bob_transport = bob_built.transport.clone();
    let alice_credentials = alice_built.credentials.clone();
    let bob_credentials = bob_built.credentials.clone();
    assert_eq!(counted(alice_built, &alice).await.total_tokens, 1);
    assert_eq!(counted(bob_built, &bob).await.total_tokens, 2);
    let alice_sent = alice_transport.request();
    let bob_sent = bob_transport.request();
    assert!(
        header(&alice_sent, "authorization")
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDALICE/")
    );
    assert!(
        header(&bob_sent, "authorization")
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDBOB/")
    );
    assert!(
        !alice_sent
            .headers
            .iter()
            .any(|(_, value)| value.contains(BOB_KEY))
    );
    assert!(
        !bob_sent
            .headers
            .iter()
            .any(|(_, value)| value.contains(ALICE_KEY))
    );
    assert_eq!(alice_credentials.calls(), 1);
    assert_eq!(bob_credentials.calls(), 1);
    let again = fixture(Reply::ok(r#"{"inputTokens": 3}"#), environment(&[]));
    let again_credentials = again.credentials.clone();
    assert_eq!(counted(again, &alice).await.total_tokens, 3);
    assert_eq!(again_credentials.calls(), 1);
}

#[tokio::test]
async fn count_call_secret_safe_failure() {
    let messages = user_messages();
    let bearer = "fixture-bearer-credential";
    let secret = "fixture-secret-access-key";
    let session = "fixture-session-credential";
    let params = deployment(&[
        ("api_key", Json::Text(bearer.to_owned())),
        ("aws_access_key_id", Json::Text("AKIDFIXTURE".to_owned())),
        ("aws_secret_access_key", Json::Text(secret.to_owned())),
        ("aws_session_token", Json::Text(session.to_owned())),
    ]);
    let (logs, guard) = capture();
    let built = fixture(
        Reply::Failure,
        environment(&[("AWS_BEARER_TOKEN_BEDROCK", bearer)]),
    );
    let environment_text = format!("{:?}", built.handler.environment);
    let credentials_text = format!("{:?}", built.credentials);
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let outcome = counter
        .count_tokens(&call(&messages, Some(&params)))
        .await
        .unwrap();
    drop(guard);
    assert!(outcome.error);
    let response_text = format!("{outcome:?}");
    let failure_text = outcome.error_message.as_deref().unwrap().to_owned();
    let log_text = logs.text();
    assert_secrets_absent(&[
        &environment_text,
        &credentials_text,
        &response_text,
        &failure_text,
        &log_text,
    ]);
    assert!(environment_text.contains("REDACTED"));
    assert!(log_text.contains("CountTokens processing error: "));
}

fn assert_secrets_absent(fields: &[&str]) {
    let secrets = [
        "fixture-bearer-credential",
        "fixture-secret-access-key",
        "fixture-session-credential",
    ];
    for field in fields {
        for secret in secrets {
            assert!(!field.contains(secret));
        }
    }
}
