use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::key_limit_policy::checks::validate_org_assignment;
use crate::key_limit_policy::error::{Entity, Rejection};
use crate::team_parent_limits::error::RateLimit;

use super::support::{FakeReads, rejected, text_detail};

#[test]
fn rejection_statuses() {
    let not_found = Rejection::ProjectNotFound {
        project_id: "proj-1".to_owned(),
    };
    assert_eq!(not_found.status(), 404);
    assert_eq!(Rejection::AssignWithoutUser.status(), 403);
    let not_member = Rejection::NotMember {
        organization_id: "org-1".to_owned(),
    };
    assert_eq!(not_member.status(), 403);
    let upperbound = Rejection::Upperbound {
        field: "max_budget",
        user_value: "2".to_owned(),
        max_value: "1".to_owned(),
    };
    assert_eq!(upperbound.status(), 400);
    let allocation = Rejection::Allocation {
        limit: RateLimit::Rpm,
        entity: Entity::Team,
        allocated: 1,
        requested: 2,
        ceiling: 3,
    };
    assert_eq!(allocation.status(), 400);
    let model = Rejection::ProjectModel {
        model: JsonData::String("claude-4".to_owned()),
        allowed: vec!["gpt-4".to_owned()],
        project_id: "proj-1".to_owned(),
    };
    assert_eq!(model.status(), 400);
    let budget = Rejection::ProjectBudget {
        requested: Decimal::from(2_i32),
        ceiling: Decimal::from(1_i32),
        project_id: "proj-1".to_owned(),
    };
    assert_eq!(budget.status(), 400);
    let non_finite = Rejection::NonFinite {
        field: "max_budget",
        received: "nan".to_owned(),
    };
    assert_eq!(non_finite.status(), 400);
}

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
