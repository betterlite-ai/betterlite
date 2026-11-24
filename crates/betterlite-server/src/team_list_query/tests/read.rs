use std::collections::BTreeMap;

use crate::team_list_query::read::{access_group_resources, key_counts_by_team, unique_ids};

use super::support::{access_group_insert, database, key_insert, record, team_insert};

#[tokio::test]
async fn batched_key_counts() {
    let seed = [
        team_insert("t1", "one", None, &[], &[]),
        team_insert("t2", "two", None, &[], &[]),
        team_insert("t3", "three", None, &[], &[]),
        key_insert("k1", "t1"),
        key_insert("k2", "t1"),
        key_insert("k3", "t2"),
        String::from("INSERT INTO \"LiteLLM_VerificationToken\" (token) VALUES ('k4');"),
    ]
    .concat();
    let fixture = database(&seed).await;
    let rows = [
        record(r#"{"team_id":"t1"}"#),
        record(r#"{"team_id":"t2"}"#),
        record(r#"{"team_id":"t1"}"#),
        record(r#"{"team_id":"t3"}"#),
    ];
    let counts = key_counts_by_team(Some(fixture.client()), &rows)
        .await
        .unwrap();
    let mut expected = BTreeMap::new();
    expected.insert("t1".to_owned(), 2_i64);
    expected.insert("t2".to_owned(), 1_i64);
    assert_eq!(counts, expected);
    fixture.close().await;
}

#[tokio::test]
async fn empty_page_no_count_query() {
    let empty = key_counts_by_team(None, &[]).await.unwrap();
    assert!(empty.is_empty());
    let rows = [
        record(r#"{"team_alias":"no id"}"#),
        record(r#"{"team_id":""}"#),
    ];
    let without_ids = key_counts_by_team(None, &rows).await.unwrap();
    assert!(without_ids.is_empty());
}

#[tokio::test]
async fn access_groups_deduplicated_missing_omitted() {
    let seed = [
        access_group_insert("g1", "one", &["m1", "m2"]),
        access_group_insert("g2", "two", &[]),
    ]
    .concat();
    let fixture = database(&seed).await;
    let wanted = vec![
        "g2".to_owned(),
        "g1".to_owned(),
        "g2".to_owned(),
        "missing".to_owned(),
    ];
    let groups = access_group_resources(Some(fixture.client()), &wanted)
        .await
        .unwrap();
    let ids: Vec<&str> = groups.keys().map(String::as_str).collect();
    assert_eq!(ids, vec!["g1", "g2"]);
    assert_eq!(groups["g1"].access_group_name, "one");
    assert_eq!(groups["g1"].access_model_names, vec!["m1", "m2"]);
    assert_eq!(groups["g2"].access_group_name, "two");
    assert!(groups["g2"].access_model_names.is_empty());
    assert_eq!(
        unique_ids(&["b".to_owned(), "a".to_owned(), "b".to_owned()]),
        vec!["b".to_owned(), "a".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn groups_without_db_empty() {
    let empty = access_group_resources(None, &[]).await.unwrap();
    assert!(empty.is_empty());
    let unwired = access_group_resources(None, &["g1".to_owned()])
        .await
        .unwrap();
    assert!(unwired.is_empty());
}
