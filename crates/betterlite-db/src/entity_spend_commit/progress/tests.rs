#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::ready;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::entity_spend_commit::progress::EntityCommitError;
use crate::entity_spend_commit::tests::support::{
    Events, Ports, RecordingCache, commit_through, transactions,
};
use crate::entity_spend_commit::{EntityTableWriter, TableFuture};
use crate::spend_commit::policy::{SpendFailure, SpendUpdateFailure};
use crate::spend_commit::tables::{SpendTable, SpendTableTransactions, spend_tables_left_to_send};

#[derive(Debug, thiserror::Error)]
#[error("scripted table failure")]
struct TableFailure(SpendFailure);

impl SpendUpdateFailure for TableFailure {
    fn spend_failure(&self) -> SpendFailure {
        self.0
    }
}

// Each write appends "write <table> <ids>" and takes the next scripted failure for its table, if any.
struct ScriptedWriter<'events> {
    events: &'events Events,
    failures: Mutex<BTreeMap<SpendTable, VecDeque<SpendFailure>>>,
}

impl<'events> ScriptedWriter<'events> {
    fn new(events: &'events Events, script: &[(SpendTable, SpendFailure)]) -> Self {
        let mut failures = BTreeMap::<SpendTable, VecDeque<SpendFailure>>::new();
        for (table, failure) in script {
            failures.entry(*table).or_default().push_back(*failure);
        }
        Self {
            events,
            failures: Mutex::new(failures),
        }
    }
}

impl EntityTableWriter for ScriptedWriter<'_> {
    type Error = TableFailure;

    fn write_table<'writer>(
        &'writer self,
        table: SpendTable,
        increments: &'writer BTreeMap<String, Decimal>,
    ) -> TableFuture<'writer, TableFailure> {
        let ids: Vec<&str> = increments.keys().map(String::as_str).collect();
        self.events
            .push(format!("write {} {}", table.name(), ids.join(",")));
        let failure = self
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&table)
            .and_then(VecDeque::pop_front);
        Box::pin(ready(
            failure.map_or(Ok(()), |kind| Err(TableFailure(kind))),
        ))
    }
}

fn every_table(rows: &[(&str, &str)]) -> SpendTableTransactions {
    let tables: Vec<(SpendTable, &[(&str, &str)])> = SpendTable::COMMIT_ORDER
        .iter()
        .map(|table| (*table, rows))
        .collect();
    transactions(&tables)
}

fn writes(events: &Events) -> Vec<String> {
    events
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("write "))
        .collect()
}

#[rstest]
#[case::absent(SpendTableTransactions::new())]
#[case::none(SpendTable::COMMIT_ORDER.iter().map(|table| (*table, None)).collect())]
#[case::empty(every_table(&[]))]
#[tokio::test]
async fn empty_categories_still_mark_all_eleven_tables(#[case] input: SpendTableTransactions) {
    let ports = Ports::new();
    let writer = ScriptedWriter::new(&ports.events, &[]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    outcome.unwrap();
    assert_eq!(committed, SpendTable::COMMIT_ORDER);
    assert!(writes(&ports.events).is_empty());
    assert_eq!(ports.failures.count(), 0);
}

#[tokio::test]
async fn every_table_commits_once_in_the_source_order() {
    let ports = Ports::new();
    let writer = ScriptedWriter::new(&ports.events, &[]);
    let input = every_table(&[
        ("team_id::b::user_id::b", "1"),
        ("team_id::a::user_id::a", "2"),
    ]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    outcome.unwrap();
    assert_eq!(committed, SpendTable::COMMIT_ORDER);
    let expected: Vec<String> = SpendTable::COMMIT_ORDER
        .iter()
        .map(|table| {
            format!(
                "write {} team_id::a::user_id::a,team_id::b::user_id::b",
                table.name()
            )
        })
        .collect();
    assert_eq!(writes(&ports.events), expected);
}

#[tokio::test]
async fn a_failed_later_table_leaves_the_committed_ones_alone() {
    let ports = Ports::new();
    let writer = ScriptedWriter::new(
        &ports.events,
        &[(SpendTable::KeyList, SpendFailure::Rejected)],
    );
    let input = every_table(&[("a", "1")]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    let error = outcome.unwrap_err();
    assert_eq!(committed, [SpendTable::UserList, SpendTable::EndUserList]);
    assert_eq!(writes(&ports.events).len(), 3);
    assert_eq!(ports.failures.count(), 1);
    let left = spend_tables_left_to_send(&input, &committed, error.spend_failure()).unwrap();
    assert_eq!(left[&SpendTable::UserList], None);
    assert_eq!(left[&SpendTable::KeyList], None);
    assert_eq!(left[&SpendTable::TeamList], input[&SpendTable::TeamList]);
}

#[rstest]
#[case::not_sent(SpendFailure::NotSent, 3, 2)]
#[case::deadlock(SpendFailure::Deadlock, 3, 2)]
#[case::ambiguous(SpendFailure::Ambiguous, 1, 0)]
#[case::rejected(SpendFailure::Rejected, 1, 0)]
#[case::other(SpendFailure::Other, 1, 0)]
#[tokio::test]
async fn only_retry_safe_failures_get_n_plus_one_attempts(
    #[case] failure: SpendFailure,
    #[case] attempts: usize,
    #[case] pauses: usize,
) {
    let ports = Ports::new();
    let script = [(SpendTable::TeamList, failure); 3];
    let writer = ScriptedWriter::new(&ports.events, &script);
    let input = transactions(&[(SpendTable::TeamList, &[("t1", "1")])]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    assert_eq!(outcome.unwrap_err().spend_failure(), failure);
    assert_eq!(writes(&ports.events).len(), attempts);
    assert_eq!(ports.pause.delays().len(), pauses);
    assert_eq!(ports.failures.count(), 1);
    assert_eq!(
        committed,
        [
            SpendTable::UserList,
            SpendTable::EndUserList,
            SpendTable::KeyList
        ]
    );
}

#[tokio::test]
async fn a_retried_table_commits_on_its_last_attempt() {
    let ports = Ports::new();
    let script = [(SpendTable::UserList, SpendFailure::Deadlock); 2];
    let writer = ScriptedWriter::new(&ports.events, &script);
    let input = transactions(&[(SpendTable::UserList, &[("u1", "1")])]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    outcome.unwrap();
    assert_eq!(committed, SpendTable::COMMIT_ORDER);
    assert_eq!(writes(&ports.events).len(), 3);
    assert_eq!(
        ports.pause.delays(),
        [Duration::from_secs(1), Duration::from_secs(2)]
    );
    assert_eq!(ports.failures.count(), 0);
}

#[rstest]
#[case::team_members(
    SpendTable::TeamMemberList,
    "team_id::t%201::user_id::u1",
    "evict team_membership:u1:t%201"
)]
#[case::projects(SpendTable::ProjectList, "p1", "evict project_id:p1")]
#[tokio::test]
async fn evictions_follow_the_committed_mark(
    #[case] table: SpendTable,
    #[case] key: &str,
    #[case] eviction: &str,
) {
    let ports = Ports::new();
    let writer = ScriptedWriter::new(&ports.events, &[]);
    let input = transactions(&[(table, &[(key, "1")])]);
    let (_committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    outcome.unwrap();
    let entries = ports.events.entries();
    let expected = [
        format!("write {} {key}", table.name()),
        format!("committed {}", table.name()),
        eviction.to_owned(),
    ];
    assert!(entries.windows(3).any(|window| window == expected));
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.starts_with("evict"))
            .count(),
        1
    );
}

#[rstest]
#[case::team_members(
    SpendTable::TeamMemberList,
    "team_id::t1::user_id::u1",
    SpendTable::OrgList
)]
#[case::projects(SpendTable::ProjectList, "p1", SpendTable::TagList)]
#[tokio::test]
async fn a_cache_failure_keeps_the_committed_table_out_of_the_replay(
    #[case] table: SpendTable,
    #[case] key: &str,
    #[case] next: SpendTable,
) {
    let ports = Ports::new();
    let writer = ScriptedWriter::new(&ports.events, &[]);
    let cache = RecordingCache {
        events: &ports.events,
        failing: true,
    };
    let input = transactions(&[(table, &[(key, "1")]), (next, &[("n1", "2")])]);
    let (committed, outcome) = commit_through(&ports, &writer, &cache, &input).await;
    let error = outcome.unwrap_err();
    assert!(matches!(error, EntityCommitError::Cache { table: failed, .. } if failed == table));
    assert_eq!(committed.last(), Some(&table));
    assert_eq!(writes(&ports.events).len(), 1);
    assert_eq!(ports.failures.count(), 0);
    let left = spend_tables_left_to_send(&input, &committed, error.spend_failure()).unwrap();
    assert_eq!(left[&table], None);
    assert_eq!(left[&next], input[&next]);
}

#[rstest]
#[case::too_few_components("team_id::t1::user_id")]
#[case::no_separator("t1")]
#[tokio::test]
async fn a_malformed_team_member_key_fails_before_any_write(#[case] key: &str) {
    let ports = Ports::new();
    let writer = ScriptedWriter::new(&ports.events, &[]);
    let input = transactions(&[(SpendTable::TeamMemberList, &[(key, "1")])]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    let error = outcome.unwrap_err();
    assert!(matches!(error, EntityCommitError::TeamMemberKey(_)));
    assert_eq!(error.spend_failure(), SpendFailure::Other);
    assert_eq!(committed.last(), Some(&SpendTable::TeamList));
    assert!(writes(&ports.events).is_empty());
    assert_eq!(ports.failures.count(), 0);
}

#[tokio::test]
async fn a_failed_member_write_evicts_nothing() {
    let ports = Ports::new();
    let writer = ScriptedWriter::new(
        &ports.events,
        &[(SpendTable::TeamMemberList, SpendFailure::Other)],
    );
    let input = transactions(&[(
        SpendTable::TeamMemberList,
        &[("team_id::t1::user_id::u1", "1")],
    )]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    assert!(matches!(outcome.unwrap_err(), EntityCommitError::Write(_)));
    assert_eq!(committed.last(), Some(&SpendTable::TeamList));
    assert!(
        !ports
            .events
            .entries()
            .iter()
            .any(|entry| entry.starts_with("evict"))
    );
}
