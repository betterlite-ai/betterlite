use secrecy::ExposeSecret as _;

use crate::auth_key_budgets::money::Money;
use crate::auth_tenant_budgets::TenantAdmission;
use crate::auth_tenant_budgets::TenantSources;
use crate::auth_tenant_budgets::error::TenantBudgetError;
use crate::auth_tenant_budgets::tags::{
    group_ceiling_applies, group_denies, max_budget_check, model_access_group_max_budget_check,
    tag_denies,
};
use crate::current_spend::CurrentSpendError;

use super::support::{
    Recorder, ScriptedSpend, TagWorld, decimal, group, number, tag, tenant_sources,
};

#[tokio::test]
async fn no_db_or_names_skips() {
    let world = TagWorld::new(
        vec![tag("t", 1.0, Some(Some(10.0)))],
        vec![group("g", 1.0, Some(Some(10.0)))],
    );
    let reader = ScriptedSpend::new(vec![number(100.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let names = ["t".to_owned()];
    let groups = ["g".to_owned()];
    let without_database = tenant_sources(&world, false);
    max_budget_check(&names, &without_database, &admission)
        .await
        .expect("tags are read only from a database");
    model_access_group_max_budget_check(&groups, &without_database, &admission)
        .await
        .expect("groups are read only from a database");
    let with_database = tenant_sources(&world, true);
    max_budget_check(&[], &with_database, &admission)
        .await
        .expect("a request without tags skips the batch read");
    model_access_group_max_budget_check(&[], &with_database, &admission)
        .await
        .expect("a request without access groups skips the batch read");
    let without_collaborators: TenantSources<'_> = TenantSources {
        database: true,
        organizations: None,
        cache: None,
        tags: None,
        groups: None,
    };
    max_budget_check(&names, &without_collaborators, &admission)
        .await
        .expect("an absent cache skips the tag check");
    model_access_group_max_budget_check(&groups, &without_collaborators, &admission)
        .await
        .expect("an absent cache skips the group check");
    assert_eq!(world.batches(), (0, 0));
    assert!(reader.reads().is_empty());
}

#[tokio::test]
async fn missing_budget_rows_skip() {
    let world = TagWorld::new(
        vec![
            tag("no-table", 5.0, None),
            tag("no-ceiling", 5.0, Some(None)),
        ],
        vec![
            group("g-no-table", 5.0, None),
            group("g-no-ceiling", 5.0, Some(None)),
        ],
    );
    let reader = ScriptedSpend::new(vec![number(100.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let sources = tenant_sources(&world, true);
    let names = [
        "absent".to_owned(),
        "no-table".to_owned(),
        "no-ceiling".to_owned(),
    ];
    max_budget_check(&names, &sources, &admission)
        .await
        .expect("a tag without a budget row or ceiling is not priced");
    let groups = [
        "g-absent".to_owned(),
        "g-no-table".to_owned(),
        "g-no-ceiling".to_owned(),
    ];
    model_access_group_max_budget_check(&groups, &sources, &admission)
        .await
        .expect("an access group without a budget row or ceiling is not priced");
    assert_eq!(world.batches(), (1, 1));
    assert!(reader.reads().is_empty());
}

#[tokio::test]
async fn authoritative_fallbacks_and_counter_names() {
    let tags_world = TagWorld::new(vec![tag("team-a", 2.0, Some(Some(10.0)))], vec![]);
    let tag_reader = ScriptedSpend::new(vec![number(7.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &tag_reader,
        alerts: Some(&recorder),
    };
    let sources = tenant_sources(&tags_world, true);
    max_budget_check(&["team-a".to_owned()], &sources, &admission)
        .await
        .expect("the tag spend stays below its ceiling");
    let reads = tag_reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:tag:team-a");
    assert_eq!(reads[0].fallback_spend, decimal("2"));
    assert_eq!(reads[0].max_budget, Some(decimal("10")));
    assert!(reads[0].fallback_authoritative);

    let groups_world = TagWorld::new(vec![], vec![group("g1", 3.0, Some(Some(10.0)))]);
    let group_reader = ScriptedSpend::new(vec![number(7.0)]);
    let admission = TenantAdmission {
        reader: &group_reader,
        alerts: Some(&recorder),
    };
    let sources = tenant_sources(&groups_world, true);
    model_access_group_max_budget_check(&["g1".to_owned()], &sources, &admission)
        .await
        .expect("the group spend stays below its ceiling");
    let reads = group_reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:model_access_group:g1");
    assert_eq!(reads[0].fallback_spend, decimal("3"));
    assert_eq!(reads[0].max_budget, Some(decimal("10")));
    assert!(reads[0].fallback_authoritative);
}

#[tokio::test]
async fn tag_equality_allowed() {
    let world = TagWorld::new(vec![tag("equal", 0.0, Some(Some(10.0)))], vec![]);
    let sources = tenant_sources(&world, true);
    let recorder = Recorder::default();
    let at_ceiling = ScriptedSpend::new(vec![number(10.0)]);
    let admission = TenantAdmission {
        reader: &at_ceiling,
        alerts: Some(&recorder),
    };
    max_budget_check(&["equal".to_owned()], &sources, &admission)
        .await
        .expect("a tag spend equal to its ceiling is allowed");

    let over = ScriptedSpend::new(vec![number(10.01)]);
    let admission = TenantAdmission {
        reader: &over,
        alerts: Some(&recorder),
    };
    let error = max_budget_check(&["equal".to_owned()], &sources, &admission)
        .await
        .expect_err("a cent over the tagged ceiling denies");
    let TenantBudgetError::Denied(denial) = error else {
        panic!("a tag denial is a BudgetExceededError");
    };
    assert_eq!(denial.entity_type, "tag");
    assert_eq!(denial.current_cost, decimal("10.01"));
    assert_eq!(denial.max_budget, decimal("10"));
}

#[tokio::test]
async fn tag_zero_negative_and_nonfinite_ceilings() {
    let world = TagWorld::new(
        vec![
            tag("zero", 0.0, Some(Some(0.0))),
            tag("negative", 0.0, Some(Some(-5.0))),
        ],
        vec![],
    );
    let sources = tenant_sources(&world, true);
    let recorder = Recorder::default();
    let reader = ScriptedSpend::new(vec![number(0.0), number(0.01), number(0.0)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    max_budget_check(&["zero".to_owned()], &sources, &admission)
        .await
        .expect("zero spend against a zero ceiling is allowed");
    let error = max_budget_check(&["zero".to_owned()], &sources, &admission)
        .await
        .expect_err("any spend against a zero ceiling denies");
    assert!(matches!(error, TenantBudgetError::Denied(_)));
    let error = max_budget_check(&["negative".to_owned()], &sources, &admission)
        .await
        .expect_err("zero spend is above a negative ceiling");
    let TenantBudgetError::Denied(denial) = error else {
        panic!("a negative tag ceiling denies");
    };
    assert_eq!(denial.max_budget, decimal("-5"));

    assert!(tag_denies(decimal("100"), Money::Nan));
    assert!(!tag_denies(decimal("100"), Money::PositiveInfinity));
    assert!(tag_denies(decimal("100"), Money::NegativeInfinity));
    assert!(!tag_denies(decimal("0"), Money::Finite(decimal("0"))));
    assert!(tag_denies(decimal("0.01"), Money::Finite(decimal("0"))));
    assert!(!tag_denies(decimal("-5"), Money::Finite(decimal("-5"))));
    assert!(tag_denies(decimal("-4"), Money::Finite(decimal("-5"))));
}

#[tokio::test]
async fn first_tag_denial_has_no_alert() {
    let world = TagWorld::new(
        vec![
            tag("over", 20.0, Some(Some(10.0))),
            tag("under", 1.0, Some(Some(10.0))),
        ],
        vec![],
    );
    let sources = tenant_sources(&world, true);
    let recorder = Recorder::default();
    let reader = ScriptedSpend::new(vec![number(20.0), number(1.0)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let error = max_budget_check(
        &["over".to_owned(), "under".to_owned()],
        &sources,
        &admission,
    )
    .await
    .expect_err("the first tag over its ceiling denies");
    let TenantBudgetError::Denied(denial) = error else {
        panic!("a tag denial is a BudgetExceededError");
    };
    assert_eq!(
        denial
            .entity_id
            .as_ref()
            .map(|entity| entity.expose_secret().to_owned()),
        Some("over".to_owned())
    );
    assert_eq!(reader.reads().len(), 1);
    assert!(recorder.take().is_empty());
}

#[tokio::test]
async fn group_nonpositive_skips_and_equality_denies() {
    let world = TagWorld::new(
        vec![],
        vec![
            group("zero", 100.0, Some(Some(0.0))),
            group("negative", 100.0, Some(Some(-1.0))),
            group("equal", 10.0, Some(Some(10.0))),
            group("below", 9.99, Some(Some(10.0))),
        ],
    );
    let sources = tenant_sources(&world, true);
    let recorder = Recorder::default();
    let reader = ScriptedSpend::new(vec![number(10.0), number(9.99)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    model_access_group_max_budget_check(
        &["zero".to_owned(), "negative".to_owned()],
        &sources,
        &admission,
    )
    .await
    .expect("a nonpositive group ceiling is not enforced");
    assert!(reader.reads().is_empty());
    let error = model_access_group_max_budget_check(&["equal".to_owned()], &sources, &admission)
        .await
        .expect_err("a group spend equal to its ceiling denies");
    let TenantBudgetError::Denied(denial) = error else {
        panic!("an access group denial is a BudgetExceededError");
    };
    assert_eq!(denial.entity_type, "model_access_group");
    assert_eq!(denial.max_budget, decimal("10"));
    model_access_group_max_budget_check(&["below".to_owned()], &sources, &admission)
        .await
        .expect("a cent below the group ceiling passes");

    assert!(!group_ceiling_applies(Money::Finite(decimal("0"))));
    assert!(!group_ceiling_applies(Money::Finite(decimal("-1"))));
    assert!(group_ceiling_applies(Money::Finite(decimal("0.01"))));
    assert!(group_ceiling_applies(Money::Nan));
    assert!(group_ceiling_applies(Money::PositiveInfinity));
    assert!(!group_ceiling_applies(Money::NegativeInfinity));
    assert!(group_denies(decimal("10"), Money::Finite(decimal("10"))));
    assert!(!group_denies(decimal("9.99"), Money::Finite(decimal("10"))));
    assert!(group_denies(decimal("100"), Money::Nan));
    assert!(!group_denies(decimal("100"), Money::PositiveInfinity));
    assert!(group_denies(decimal("100"), Money::NegativeInfinity));
}

#[tokio::test]
async fn only_matched_groups_checked() {
    let world = TagWorld::new(
        vec![],
        vec![
            group("matched", 1.0, Some(Some(10.0))),
            group("unmatched", 100.0, Some(Some(1.0))),
        ],
    );
    let sources = tenant_sources(&world, true);
    let recorder = Recorder::default();
    let reader = ScriptedSpend::new(vec![number(1.0), number(100.0)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    model_access_group_max_budget_check(&["matched".to_owned()], &sources, &admission)
        .await
        .expect("an over-budget group the request matched nothing in is not priced");
    let reads = reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:model_access_group:matched");
    let error =
        model_access_group_max_budget_check(&["unmatched".to_owned()], &sources, &admission)
            .await
            .expect_err("the same group denies once the request matches it");
    let TenantBudgetError::Denied(denial) = error else {
        panic!("an access group denial is a BudgetExceededError");
    };
    assert_eq!(denial.max_budget, decimal("1"));
    assert_eq!(world.batches(), (0, 2));
}

#[tokio::test]
async fn counter_failure_propagates() {
    let world = TagWorld::new(
        vec![tag("t", 1.0, Some(Some(10.0)))],
        vec![group("g", 1.0, Some(Some(10.0)))],
    );
    let sources = tenant_sources(&world, true);
    let recorder = Recorder::default();
    let failing_tags = ScriptedSpend::failing(vec![], 0);
    let admission = TenantAdmission {
        reader: &failing_tags,
        alerts: Some(&recorder),
    };
    let error = max_budget_check(&["t".to_owned()], &sources, &admission)
        .await
        .expect_err("an unverifiable tag counter fails the request");
    let TenantBudgetError::Spend(CurrentSpendError::Unverifiable { counter_key }) = error else {
        panic!("a failed counter read stays a spend failure");
    };
    assert_eq!(counter_key, "spend:tag:t");

    let failing_groups = ScriptedSpend::failing(vec![], 0);
    let admission = TenantAdmission {
        reader: &failing_groups,
        alerts: Some(&recorder),
    };
    let error = model_access_group_max_budget_check(&["g".to_owned()], &sources, &admission)
        .await
        .expect_err("an unverifiable group counter fails the request");
    let TenantBudgetError::Spend(CurrentSpendError::Unverifiable { counter_key }) = error else {
        panic!("a failed counter read stays a spend failure");
    };
    assert_eq!(counter_key, "spend:model_access_group:g");
}

#[tokio::test]
async fn read_only_concurrent_checks_can_overshoot() {
    let world = TagWorld::new(vec![tag("t", 0.0, Some(Some(10.0)))], vec![]);
    let sources = tenant_sources(&world, true);
    let recorder = Recorder::default();
    let reader = ScriptedSpend::new(vec![number(6.0), number(6.0)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let names = ["t".to_owned()];
    let (first, second) = tokio::join!(
        max_budget_check(&names, &sources, &admission),
        max_budget_check(&names, &sources, &admission),
    );
    first.expect("the first concurrent read stays below the ceiling");
    second.expect("the second read sees the same spend and also passes");
    let reads = reader.reads();
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0].counter_key, reads[1].counter_key);
    assert_eq!(reads[0].fallback_spend, decimal("0"));
    assert_eq!(reads[1].fallback_spend, decimal("0"));
}
