use std::fs;

use super::support::{Fixture, answer, token_reply};
use crate::workload_identity::config::OpenAiWorkloadIdentityConfig;
use crate::workload_identity::error::WorkloadIdentityError;

const SUBJECT: &str = "subject-secret-jwt";
const ACCESS: &str = "access-secret-token";

async fn tokens_for(fixture: &Fixture, configs: &[&OpenAiWorkloadIdentityConfig]) -> Vec<String> {
    let mut tokens = Vec::new();
    for config in configs {
        tokens.push(fixture.token(config).await);
    }
    tokens
}

fn assert_hides(failure: &WorkloadIdentityError, hidden: &[&str]) {
    for text in [format!("{failure}"), format!("{failure:?}")] {
        for secret in hidden {
            assert!(!text.contains(secret), "{text}");
        }
    }
}

#[tokio::test]
async fn openai_workload_identity_credential_identity_isolation() {
    let fixture = Fixture::new(vec![
        token_reply("token-a", "3600"),
        token_reply("token-b", "3600"),
        token_reply("token-c", "3600"),
        token_reply("token-d", "3600"),
    ]);
    let base = fixture.config("shared", "subject");
    let other_idp = OpenAiWorkloadIdentityConfig {
        identity_provider_id: "idp-other".to_owned(),
        ..base.clone()
    };
    let other_account = OpenAiWorkloadIdentityConfig {
        service_account_id: "sa-other".to_owned(),
        ..base.clone()
    };
    let other_file = fixture.config("other-file", "subject");
    let configs = [&base, &other_idp, &other_account, &other_file];
    let first = tokens_for(&fixture, &configs).await;
    assert_eq!(first, ["token-a", "token-b", "token-c", "token-d"]);
    assert_eq!(fixture.script.sent(), 4);
    assert_eq!(tokens_for(&fixture, &configs).await, first);
    assert_eq!(fixture.script.sent(), 4);
}

#[tokio::test]
async fn openai_workload_identity_secret_safe_failure() {
    let granted = Fixture::new(vec![token_reply(ACCESS, "3600")]);
    let granted_config = granted.config("secret", SUBJECT);
    let token = granted.bearer(&granted_config).await.unwrap();
    assert!(!format!("{token:?}").contains(ACCESS));
    assert!(granted.script.take()[0].body.contains(SUBJECT));

    let denied = Fixture::new(vec![answer(400, r#"{"error":"invalid_grant"}"#)]);
    let denied_config = denied.config("secret", SUBJECT);
    let failure = denied.bearer(&denied_config).await.unwrap_err();
    assert!(matches!(failure, WorkloadIdentityError::OAuth { .. }));
    assert_hides(&failure, &[SUBJECT, ACCESS]);

    let blank_config = denied.config("blank", SUBJECT);
    fs::write(&blank_config.token_file, "   ").unwrap();
    let blank = denied.bearer(&blank_config).await.unwrap_err();
    assert_hides(&blank, &[SUBJECT]);
}
