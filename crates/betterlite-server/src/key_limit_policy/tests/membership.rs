use crate::key_limit_policy::checks::validate_org_assignment;
use crate::key_limit_policy::error::Rejection;

use super::support::{FakeReads, rejected, text_detail};

#[tokio::test]
async fn org_assignment_denied() {
    let reads = FakeReads::default();
    let rejection = rejected(validate_org_assignment(None, "org-1", &reads).await);
    assert_eq!(
        text_detail(&rejection),
        "Cannot assign a key to an organization without a user_id on the caller's token"
    );
    assert!(matches!(rejection, Rejection::AssignWithoutUser));
    assert!(reads.user_lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn caller_org_membership() {
    let mut reads = FakeReads::default();
    reads.memberships.insert("org-1".to_owned());
    validate_org_assignment(Some("u1"), "org-1", &reads)
        .await
        .unwrap();
    assert_eq!(*reads.user_lookups.lock().unwrap(), vec!["u1".to_owned()]);
    let rejection = rejected(validate_org_assignment(Some("u1"), "org-2", &reads).await);
    assert_eq!(
        text_detail(&rejection),
        "Caller is not a member of organization_id=org-2"
    );
    assert!(matches!(rejection, Rejection::NotMember { .. }));
}

#[tokio::test]
async fn caller_org_empty_memberships() {
    let reads = FakeReads::default();
    let rejection = rejected(validate_org_assignment(Some("u1"), "org-1", &reads).await);
    assert_eq!(
        text_detail(&rejection),
        "Caller is not a member of organization_id=org-1"
    );
}

#[tokio::test]
async fn caller_org_no_admin_shortcut() {
    let reads = FakeReads::default();
    let rejection = rejected(validate_org_assignment(Some("proxy-admin"), "org-1", &reads).await);
    assert!(matches!(rejection, Rejection::NotMember { .. }));
}
