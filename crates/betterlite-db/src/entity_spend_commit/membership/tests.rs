#![cfg(test)]

use std::str::FromStr as _;
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;
use tokio::time::sleep;
use tokio_postgres::Client;

use super::{
    MemberKeyError, MemberSpendRow, OrgMemberSpend, TeamMemberKey, org_member_rows,
    team_lock_order, team_member_key, team_member_rows,
};
use crate::entity_spend_commit::progress::EntityCommitError;
use crate::entity_spend_commit::tests::support::{
    Database, MEMBER_SPEND, MEMBER_TOTAL_SPEND, Ports, SEED_SQL, amount, commit_through,
    increments, transactions,
};
use crate::entity_spend_commit::{EntityTableError, EntityTableWriter as _};
use crate::spend_commit::policy::{SpendFailure, SpendUpdateFailure};
use crate::spend_commit::tables::SpendTable;

const ORG_MEMBER_SPEND: &str = r#"SELECT spend::numeric FROM "LiteLLM_OrganizationMembership" WHERE organization_id || '/' || user_id = $1"#;
const ROSTER_SQL: &str = r#"
INSERT INTO "LiteLLM_TeamTable" (team_id, members_with_roles) VALUES
    ('a', '[{"user_id": "u1", "role": "user"}, {"user_id": "u2", "role": "user"}]'),
    ('a-x', '[{"user_id": "u1", "role": "admin"}]');
"#;
const ADVISORY_WAITERS_SQL: &str = "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted AND database = (SELECT oid FROM pg_database WHERE datname = current_database())";

#[rstest]
#[case::four_components("team_id::t1::user_id::u1", "t1", "u1")]
#[case::ids_stay_encoded("team_id::t%3A1::user_id::u%201", "t%3A1", "u%201")]
#[case::extra_components_are_ignored("team_id::t1::user_id::u1::tail", "t1", "u1")]
#[case::empty_ids("team_id::::user_id::", "", "")]
fn team_member_keys_read_the_raw_second_and_fourth_components(
    #[case] key: &str,
    #[case] team_id: &str,
    #[case] user_id: &str,
) {
    assert_eq!(team_member_key(key), Ok(TeamMemberKey { team_id, user_id }));
}

#[rstest]
#[case::three_components("team_id::t1::user_id")]
#[case::two_components("team_id::t1")]
#[case::no_separator("t1")]
fn team_member_keys_without_a_fourth_component_are_refused(#[case] key: &str) {
    let expected = MemberKeyError {
        key: key.to_owned(),
    };
    assert_eq!(team_member_key(key), Err(expected));
}

#[rstest]
#[case::plain("organization_id::o1::user_id::u1", "o1", "u1")]
#[case::separator_in_an_id("organization_id::o%3A%3A1::user_id::u%201", "o::1", "u 1")]
#[case::invalid_utf8("organization_id::o%FF::user_id::u1", "o\u{fffd}", "u1")]
#[case::broken_escape("organization_id::o%zz::user_id::u%2", "o%zz", "u%2")]
#[case::plus_stays_plus("organization_id::o+1::user_id::u1", "o+1", "u1")]
fn org_member_keys_decode_like_unquote(
    #[case] key: &str,
    #[case] organization_id: &str,
    #[case] user_id: &str,
) {
    let rows = org_member_rows(&increments(&[(key, "1")])).unwrap();
    let expected = OrgMemberSpend {
        organization_id: organization_id.to_owned(),
        user_id: user_id.to_owned(),
        cost: Decimal::ONE,
    };
    assert_eq!(rows, [expected]);
}

#[rstest]
#[case::three_components("organization_id::o1::user_id")]
#[case::five_components("organization_id::o1::user_id::u1::tail")]
fn org_member_keys_need_exactly_four_components(#[case] key: &str) {
    let refused = org_member_rows(&increments(&[
        ("organization_id::o0::user_id::u0", "1"),
        (key, "1"),
    ]));
    let expected = MemberKeyError {
        key: key.to_owned(),
    };
    assert_eq!(refused, Err(expected));
}

#[test]
fn member_rows_sort_by_team_then_user_and_lock_each_team_once() {
    let input = increments(&[
        ("team_id::a-x::user_id::u1", "1"),
        ("team_id::a::user_id::u2", "2"),
        ("team_id::a::user_id::u1", "3"),
    ]);
    let write = team_member_rows(&input).unwrap();
    let order: Vec<(&str, &str)> = write
        .rows
        .iter()
        .map(|row| (row.team_id, row.user_id))
        .collect();
    assert_eq!(order, [("a", "u1"), ("a", "u2"), ("a-x", "u1")]);
    assert_eq!(write.teams, ["a", "a-x"]);
    assert_eq!(team_lock_order(&write.rows), ["a", "a-x"]);
}

#[test]
fn member_rows_carry_exact_decimal_costs() {
    let rows = [MemberSpendRow {
        user_id: "u1",
        team_id: "t1",
        cost: Decimal::from_str("0.1000000000000000055").unwrap(),
    }];
    assert_eq!(
        serde_json::to_string(&rows).unwrap(),
        r#"[{"user_id":"u1","team_id":"t1","cost":0.1000000000000000055}]"#
    );
}

#[tokio::test]
async fn a_rostered_member_is_created_with_spend_and_total_spend() {
    let database = Database::open(1).await;
    database.seed(ROSTER_SQL).await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[
        ("team_id::a::user_id::u2", "0.5"),
        ("team_id::a-x::user_id::u2", "0.5"),
    ]);
    writer
        .write_table(SpendTable::TeamMemberList, &rows)
        .await
        .unwrap();
    assert_eq!(database.spend(MEMBER_SPEND, "a/u2").await, amount("0.5"));
    assert_eq!(
        database.spend(MEMBER_TOTAL_SPEND, "a/u2").await,
        amount("0.5")
    );
    assert_eq!(database.spend(MEMBER_SPEND, "a-x/u2").await, None);
    database.close().await;
}

#[tokio::test]
async fn an_existing_membership_still_increments_after_removal() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    database
        .seed(r#"UPDATE "LiteLLM_TeamTable" SET members_with_roles = '[]' WHERE team_id = 't1'"#)
        .await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[("team_id::t1::user_id::u1", "0.5")]);
    writer
        .write_table(SpendTable::TeamMemberList, &rows)
        .await
        .unwrap();
    assert_eq!(database.spend(MEMBER_SPEND, "t1/u1").await, amount("1.5"));
    assert_eq!(
        database.spend(MEMBER_TOTAL_SPEND, "t1/u1").await,
        amount("3.5")
    );
    database.close().await;
}

async fn wait_for_an_advisory_waiter(client: &Client) {
    for _poll in 0_u32..400 {
        let waiting: i64 = client
            .query_one(ADVISORY_WAITERS_SQL, &[])
            .await
            .unwrap()
            .get(0);
        if waiting > 0 {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("the spend write never waited on a team lock");
}

#[tokio::test]
async fn a_member_removed_while_the_write_waits_is_not_recreated() {
    let mut database = Database::open(1).await;
    database.seed(ROSTER_SQL).await;
    let mut remover = database.session().await;
    let removal = remover.transaction().await.unwrap();
    removal
        .batch_execute(r#"SELECT pg_advisory_xact_lock(hashtext('a')); UPDATE "LiteLLM_TeamTable" SET members_with_roles = '[]' WHERE team_id = 'a'"#)
        .await
        .unwrap();
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[("team_id::a::user_id::u1", "0.5")]);
    let release = async {
        wait_for_an_advisory_waiter(&database.client).await;
        removal.commit().await.unwrap();
    };
    let (written, ()) = tokio::join!(
        writer.write_table(SpendTable::TeamMemberList, &rows),
        release
    );
    written.unwrap();
    assert_eq!(database.spend(MEMBER_SPEND, "a/u1").await, None);
    database.close().await;
}

#[tokio::test]
async fn team_locks_are_taken_in_sorted_team_order() {
    let mut database = Database::open(1).await;
    database.seed(ROSTER_SQL).await;
    let mut holder = database.session().await;
    let held = holder.transaction().await.unwrap();
    held.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&"a-x"])
        .await
        .unwrap();
    let prober = database.session().await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[
        ("team_id::a-x::user_id::u1", "1"),
        ("team_id::a::user_id::u1", "1"),
    ]);
    let probe = async {
        wait_for_an_advisory_waiter(&database.client).await;
        let team_a_free: bool = prober
            .query_one("SELECT pg_try_advisory_xact_lock(hashtext($1))", &[&"a"])
            .await
            .unwrap()
            .get(0);
        held.commit().await.unwrap();
        team_a_free
    };
    let (written, team_a_free) =
        tokio::join!(writer.write_table(SpendTable::TeamMemberList, &rows), probe);
    written.unwrap();
    assert!(!team_a_free);
    assert_eq!(database.spend(MEMBER_SPEND, "a-x/u1").await, amount("1"));
    database.close().await;
}

#[tokio::test]
async fn a_malformed_org_member_key_takes_the_ordinary_failure_path() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let input = transactions(&[(
        SpendTable::OrgMemberList,
        &[
            ("organization_id::o%201::user_id::u%201", "1"),
            ("organization_id::o1", "1"),
        ],
    )]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        EntityCommitError::Write(EntityTableError::Key { .. })
    ));
    assert_eq!(error.spend_failure(), SpendFailure::Other);
    assert_eq!(committed.last(), Some(&SpendTable::OrgList));
    assert!(ports.pause.delays().is_empty());
    assert_eq!(ports.failures.count(), 1);
    assert_eq!(
        database.spend(ORG_MEMBER_SPEND, "o 1/u 1").await,
        amount("1")
    );
    database.close().await;
}

#[tokio::test]
async fn a_null_org_membership_spend_stays_null() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    database
        .seed(r#"UPDATE "LiteLLM_OrganizationMembership" SET spend = NULL"#)
        .await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[("organization_id::o%201::user_id::u%201", "1")]);
    writer
        .write_table(SpendTable::OrgMemberList, &rows)
        .await
        .unwrap();
    let still_null: bool = database
        .client
        .query_one(
            r#"SELECT spend IS NULL FROM "LiteLLM_OrganizationMembership" WHERE user_id = 'u 1'"#,
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(still_null);
    database.close().await;
}
