use crate::assume_role::is_already_running_as_role;

use super::fixture::{Reply, sources};
use super::logs::capture;
use super::probe::{AMBIENT, SAME_ROLE, TARGET};

const DENIED: &str = "<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>nope</Message></Error><RequestId>r</RequestId></ErrorResponse>";
const NO_ARN: &str = "<GetCallerIdentityResponse><GetCallerIdentityResult></GetCallerIdentityResult></GetCallerIdentityResponse>";
const FAILURE_LOG: &str = "Could not determine current role identity:";

pub(super) async fn probe_with_log(environment: &[(&str, &str)], reply: Reply) -> (bool, String) {
    let (logs, _guard) = capture();
    let (native, recorder) = sources(environment, reply);
    let running = is_already_running_as_role(TARGET, &native).await;
    assert_eq!(recorder.count("AssumeRole"), 0);
    (running, logs.text())
}

async fn check(environment: &[(&str, &str)], reply: Reply, running: bool, line: &str) {
    let (actual, text) = probe_with_log(environment, reply).await;
    assert_eq!(actual, running, "{reply:?}");
    assert!(text.contains(line), "{reply:?}: {text}");
}

pub(super) async fn sts_error() {
    let matched = format!("Current identity already matches target role: {TARGET}");
    let failures = [
        Reply::Raw(403, DENIED),
        Reply::Refused,
        Reply::Raw(200, "<not xml"),
    ];
    for reply in failures {
        check(&AMBIENT, reply, false, FAILURE_LOG).await;
    }
    let identity = Reply::Identity(SAME_ROLE);
    check(&AMBIENT[..2], identity, true, &matched).await;
    let (native, recorder) = sources(&AMBIENT[..2], identity);
    assert!(is_already_running_as_role(TARGET, &native).await);
    assert!(
        recorder
            .authorizations()
            .concat()
            .contains("/us-east-1/sts/aws4_request")
    );
    check(&AMBIENT, identity, true, &matched).await;
    let (running, text) = probe_with_log(&AMBIENT, Reply::Raw(200, NO_ARN)).await;
    assert!(!running);
    assert!(!text.contains(FAILURE_LOG));
}
