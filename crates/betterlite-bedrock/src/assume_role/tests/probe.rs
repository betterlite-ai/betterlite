use crate::assume_role::is_already_running_as_role;

use super::fixture::{Reply, sources};

pub(super) const AMBIENT: [(&str, &str); 3] = [
    ("AWS_ACCESS_KEY_ID", "AKIAFIXTUREA"),
    ("AWS_SECRET_ACCESS_KEY", "fixture-secret-a"),
    ("AWS_REGION", "us-east-1"),
];
pub(super) const TARGET: &str = "arn:aws:iam::123456789012:role/team/Target";
pub(super) const SAME_ROLE: &str = "arn:aws:sts::123456789012:assumed-role/Target/session";
const IRSA: [(&str, &str); 2] = [
    ("AWS_ROLE_ARN", TARGET),
    ("AWS_WEB_IDENTITY_TOKEN_FILE", "/missing/token"),
];

pub(super) async fn already_role_no_second_assume() {
    for target in ["not-an-arn", "arn:aws:iam::123:user/Target"] {
        let (native, recorder) =
            sources(&[IRSA[0], IRSA[1], AMBIENT[0]], Reply::Identity(SAME_ROLE));
        assert!(!is_already_running_as_role(target, &native).await);
        assert_eq!(recorder.count("GetCallerIdentity"), 0);
    }
    let (native, recorder) = sources(&IRSA, Reply::Refused);
    assert!(is_already_running_as_role(TARGET, &native).await);
    let other = "arn:aws:iam::123456789012:role/Target";
    assert!(!is_already_running_as_role(other, &native).await);
    assert_eq!(recorder.count("GetCallerIdentity"), 0);
    for partial in [
        vec![IRSA[0], AMBIENT[0], AMBIENT[1], AMBIENT[2]],
        vec![IRSA[1], AMBIENT[0], AMBIENT[1], AMBIENT[2]],
        vec![
            ("AWS_ROLE_ARN", ""),
            IRSA[1],
            AMBIENT[0],
            AMBIENT[1],
            AMBIENT[2],
        ],
    ] {
        let (ambient, calls) = sources(&partial, Reply::Identity(SAME_ROLE));
        assert!(is_already_running_as_role(TARGET, &ambient).await);
        assert_eq!(calls.count("GetCallerIdentity"), 1);
        assert_eq!(calls.count("AssumeRole"), 0);
    }
}

pub(super) async fn different_account_role_chain() {
    for identity in [
        "arn:aws:sts::999999999999:assumed-role/Target/session",
        "arn:aws-cn:sts::123456789012:assumed-role/Target/session",
        "arn:aws:sts::123456789012:assumed-role/Other/session",
    ] {
        let (native, recorder) = sources(&AMBIENT, Reply::Identity(identity));
        assert!(
            !is_already_running_as_role(TARGET, &native).await,
            "{identity}"
        );
        assert_eq!(recorder.count("GetCallerIdentity"), 1);
        assert_eq!(recorder.count("AssumeRole"), 0);
    }
}
