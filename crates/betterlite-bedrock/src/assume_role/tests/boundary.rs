use crate::assume_role::{expiry::StsResponse, is_already_running_as_role};

use super::expiry::sts_json;
use super::failure::probe_with_log;
use super::fixture::{Reply, sources};
use super::probe::{SAME_ROLE, TARGET};

const SECRET: &str = "fixture-secret-never-log";
const TOKEN: &str = "fixture-token-never-log";
const DENIED: &str = "<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>nope</Message></Error><RequestId>r</RequestId></ErrorResponse>";

#[tokio::test]
async fn bedrock_assume_role_credential_identity_isolation() {
    let first = [
        ("AWS_ACCESS_KEY_ID", "AKIAFIXTUREA"),
        ("AWS_SECRET_ACCESS_KEY", "a"),
        ("AWS_REGION", "us-east-1"),
    ];
    let second = [
        ("AWS_ACCESS_KEY_ID", "AKIAFIXTUREB"),
        ("AWS_SECRET_ACCESS_KEY", "b"),
        ("AWS_REGION", "us-east-1"),
    ];
    let (native_first, recorder_first) = sources(&first, Reply::Identity(SAME_ROLE));
    let (native_second, recorder_second) = sources(
        &second,
        Reply::Identity("arn:aws:sts::999:assumed-role/Target/s"),
    );
    assert!(is_already_running_as_role(TARGET, &native_first).await);
    assert!(!is_already_running_as_role(TARGET, &native_second).await);
    assert!(is_already_running_as_role(TARGET, &native_first).await);
    let signed_first = recorder_first.authorizations().join(" ");
    let signed_second = recorder_second.authorizations().join(" ");
    assert!(signed_first.contains("AKIAFIXTUREA") && !signed_first.contains("AKIAFIXTUREB"));
    assert!(signed_second.contains("AKIAFIXTUREB") && !signed_second.contains("AKIAFIXTUREA"));
}

#[tokio::test]
async fn bedrock_assume_role_secret_safe_failure() {
    let environment = [
        ("AWS_ACCESS_KEY_ID", "AKIAFIXTURE"),
        ("AWS_SECRET_ACCESS_KEY", SECRET),
        ("AWS_SESSION_TOKEN", TOKEN),
        ("AWS_REGION", "us-east-1"),
    ];
    let (running, text) = probe_with_log(&environment, Reply::Raw(403, DENIED)).await;
    assert!(!running);
    assert!(text.contains("Could not determine current role identity:"));
    assert!(!text.contains(SECRET) && !text.contains(TOKEN));
    let ours = text
        .lines()
        .filter(|line| line.contains("current role identity"));
    assert!(ours.into_iter().all(|line| !line.contains("AKIAFIXTURE")));
    let good = sts_json(["AKIAFIXTURE", SECRET, TOKEN, "2026-01-01T01:00:00Z"], "");
    let debug = format!("{:?}", serde_json::from_str::<StsResponse>(&good).unwrap());
    assert!(!debug.contains(SECRET) && !debug.contains(TOKEN) && !debug.contains("AKIAFIXTURE"));
    let bad = sts_json([SECRET, SECRET, TOKEN, TOKEN], "");
    let error = serde_json::from_str::<StsResponse>(&bad).unwrap_err();
    assert!(
        !error.to_string().contains(TOKEN) && !error.to_string().contains(SECRET),
        "{error}"
    );
}
