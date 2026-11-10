use std::fs;

use super::support::{Fixture, Reply, answer, token_reply};
use crate::workload_identity::error::{TokenFileError, WorkloadIdentityError};
use crate::workload_identity::token_file::MAX_TOKEN_FILE_BYTES;

type OAuthFields = (u16, Option<String>, String);

const FALLBACK: &str = "OAuth authentication error.";

pub(super) async fn empty_subject_token() {
    blank_token_files().await;
    unreadable_token_files().await;
    token_file_limit().await;
}

async fn blank_token_files() {
    let fixture = Fixture::new(vec![token_reply("access", "3600")]);
    let contents = [
        ("blank", " \n\t "),
        ("empty", ""),
        ("controls", "\u{1c}\u{1d}\u{1e}\u{1f}"),
    ];
    for (name, content) in contents {
        let config = fixture.config(name, content);
        let outcome = fixture.bearer(&config).await;
        assert!(
            matches!(
                outcome,
                Err(WorkloadIdentityError::TokenFile {
                    source: TokenFileError::Empty { .. },
                    ..
                })
            ),
            "{name}"
        );
    }
    assert_eq!(fixture.script.sent(), 0);
}

async fn unreadable_token_files() {
    let fixture = Fixture::new(vec![token_reply("access", "3600")]);
    let missing = fixture.config("missing", "x");
    fs::remove_file(&missing.token_file).unwrap();
    let io_failure = fixture.outcome(&missing).await;
    assert!(io_failure.starts_with("Failed to read the token file at "));
    assert!(io_failure.contains("missing"));
    let binary = fixture.config("binary", "");
    fs::write(&binary.token_file, [0xff_u8, 0xfe_u8]).unwrap();
    let not_utf8 = fixture.outcome(&binary).await;
    assert!(not_utf8.ends_with("is not valid UTF-8"), "{not_utf8}");
    let oversized = fixture.config(
        "oversized",
        &"a".repeat(MAX_TOKEN_FILE_BYTES.saturating_add(1)),
    );
    let too_large = fixture.outcome(&oversized).await;
    assert!(
        too_large.contains("is larger than 1048576 bytes"),
        "{too_large}"
    );
    assert_eq!(fixture.script.sent(), 0);
}

async fn token_file_limit() {
    let fixture = Fixture::new(vec![token_reply("access", "3600")]);
    let at_limit = fixture.config("at-limit", &"a".repeat(MAX_TOKEN_FILE_BYTES));
    assert_eq!(fixture.token(&at_limit).await, "access");
    assert_eq!(fixture.script.sent(), 1);
}

async fn failures_for(replies: Vec<Reply>) -> Vec<WorkloadIdentityError> {
    let fixture = Fixture::new(replies);
    let config = fixture.config("oauth", "subject");
    let mut collected = Vec::new();
    while fixture.script.pending() > 0 {
        let failure = fixture.bearer(&config).await.unwrap_err();
        collected.push(failure);
    }
    collected
}

fn oauth_fields(failure: &WorkloadIdentityError) -> Option<OAuthFields> {
    if let WorkloadIdentityError::OAuth {
        status,
        error,
        message,
    } = failure
    {
        return Some((*status, error.clone(), message.clone()));
    }
    None
}

pub(super) async fn oauth_status_errors() {
    let described = r#"{"error":"invalid_grant","error_description":"subject expired"}"#;
    let oauth = failures_for(vec![
        answer(400, described),
        answer(401, r#"{"error":"invalid_client"}"#),
        answer(403, "not json"),
        answer(403, r#"{"error":7,"error_description":""}"#),
        answer(400, ""),
    ])
    .await;
    let fields: Vec<_> = oauth.iter().map(oauth_fields).collect();
    let expected = [
        (400, Some("invalid_grant"), "subject expired"),
        (401, Some("invalid_client"), FALLBACK),
        (403, None, FALLBACK),
        (403, None, FALLBACK),
        (400, None, FALLBACK),
    ];
    for (found, (status, error, message)) in fields.into_iter().zip(expected) {
        let wanted = (status, error.map(str::to_owned), message.to_owned());
        assert_eq!(found, Some(wanted));
    }
    other_statuses(described).await;
}

async fn other_statuses(described: &str) {
    let rest = failures_for(vec![
        answer(404, described),
        answer(500, described),
        answer(302, ""),
        answer(199, ""),
    ])
    .await;
    let texts: Vec<_> = rest.iter().map(ToString::to_string).collect();
    assert_eq!(
        texts,
        [
            "Token exchange failed with status 404",
            "Token exchange failed with status 500",
            "Token exchange failed with status 302",
            "Token exchange failed with status 199",
        ]
    );
    assert!(rest.iter().all(|failure| oauth_fields(failure).is_none()));
}
