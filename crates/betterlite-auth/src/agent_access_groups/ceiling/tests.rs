#![cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::ready;
use std::sync::{Arc, Mutex};

use betterlite_db::resource_rows::access_group::AccessGroupTable;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::{AgentAccessGroupCeiling, AgentAccessGroups};
use crate::agent_access_groups::error::AccessGroupsError;
use crate::agent_access_groups::reads::AccessGroupReads;
use crate::auth_team_lookup::error::{TeamFailure, TeamLookupError};

#[derive(Clone)]
enum Stored {
    Row(AccessGroupTable),
    Missing,
    PolicyDown,
    Plain,
}

struct FakeReads {
    database: bool,
    agents: BTreeMap<String, Vec<String>>,
    groups: BTreeMap<String, Stored>,
    reads: Mutex<Vec<(String, bool)>>,
}

impl AccessGroupReads for FakeReads {
    fn has_database(&self) -> bool {
        self.database
    }

    fn agent_access_group_ids(&self, agent_id: &str) -> impl Future<Output = Vec<String>> + Send {
        ready(self.agents.get(agent_id).cloned().unwrap_or_default())
    }

    fn access_group(
        &self,
        access_group_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<AccessGroupTable, TeamLookupError>> + Send {
        self.reads
            .lock()
            .unwrap()
            .push((access_group_id.to_owned(), check_db_only));
        let id = access_group_id.to_owned();
        ready(match self.groups.get(access_group_id).cloned() {
            Some(Stored::Row(table)) => Ok(table),
            Some(Stored::Missing) | None => Err(TeamLookupError::AccessGroupNotFound {
                access_group_id: id,
            }),
            Some(Stored::PolicyDown) => Err(TeamLookupError::AccessPolicyUnavailable {
                access_group_id: id,
                cause: Box::new(TeamFailure::Throttled),
            }),
            Some(Stored::Plain) => Err(TeamLookupError::NoDatabase),
        })
    }
}

#[derive(Clone, Default)]
struct Warnings {
    messages: Arc<Mutex<Vec<String>>>,
}

impl Warnings {
    fn install(&self) -> DefaultGuard {
        let layer = WarningLayer {
            messages: Arc::clone(&self.messages),
        };
        set_default(tracing_subscriber::registry().with(layer))
    }

    fn messages(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }
}

struct WarningLayer {
    messages: Arc<Mutex<Vec<String>>>,
}

impl<S: Subscriber> Layer<S> for WarningLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = MessageField::default();
        event.record(&mut message);
        self.messages.lock().unwrap().push(message.text);
    }
}

#[derive(Default)]
struct MessageField {
    text: String,
}

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.text = format!("{value:?}");
        }
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn row(id: &str, models: &[&str], servers: &[&str], agents: &[&str]) -> Stored {
    Stored::Row(AccessGroupTable {
        access_group_id: id.to_owned(),
        access_group_name: format!("{id} name"),
        description: None,
        access_model_names: strings(models),
        access_mcp_server_ids: strings(servers),
        access_agent_ids: strings(agents),
        assigned_team_ids: Vec::new(),
        assigned_key_ids: Vec::new(),
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    })
}

fn groups(
    database: bool,
    attached: &[&str],
    stored: &[(&str, Stored)],
) -> AgentAccessGroups<FakeReads> {
    AgentAccessGroups {
        reads: FakeReads {
            database,
            agents: BTreeMap::from([("agent-1".to_owned(), strings(attached))]),
            groups: stored
                .iter()
                .map(|(id, group)| ((*id).to_owned(), group.clone()))
                .collect(),
            reads: Mutex::new(Vec::new()),
        },
    }
}

fn reads(access: &AgentAccessGroups<FakeReads>) -> Vec<(String, bool)> {
    access.reads.reads.lock().unwrap().clone()
}

fn two_groups() -> Vec<(&'static str, Stored)> {
    vec![
        (
            "g1",
            row("g1", &["gpt-4o", "claude"], &["srv-b"], &["agent-x"]),
        ),
        (
            "g2",
            row("g2", &["claude", "gemini"], &["srv-a"], &["agent-y"]),
        ),
    ]
}

#[rstest]
#[case::unknown_agent("agent-2")]
#[case::agent_with_an_empty_list("agent-1")]
#[tokio::test]
async fn no_attached_group_caps_nothing_and_reads_no_group(#[case] agent_id: &str) {
    let access = groups(true, &[], &two_groups());
    let ceiling = access.resolve_agent_ceiling(agent_id, false).await.unwrap();
    assert_eq!(ceiling, None);
    assert!(reads(&access).is_empty());
}

#[tokio::test]
async fn attached_groups_union_their_lists_and_keep_the_agent_order() {
    let access = groups(true, &["g2", "g1", "g2"], &two_groups());
    let ceiling = access
        .resolve_agent_ceiling("agent-1", false)
        .await
        .unwrap();
    let expected = AgentAccessGroupCeiling {
        access_group_ids: strings(&["g2", "g1", "g2"]),
        models: set(&["claude", "gemini", "gpt-4o"]),
        mcp_server_ids: set(&["srv-a", "srv-b"]),
        agent_ids: set(&["agent-x", "agent-y"]),
    };
    assert_eq!(ceiling, Some(expected));
    let lenient = vec![
        ("g2".to_owned(), false),
        ("g1".to_owned(), false),
        ("g2".to_owned(), false),
    ];
    assert_eq!(reads(&access), lenient);
}

#[rstest]
#[case::row_missing(Stored::Missing)]
#[case::policy_unavailable(Stored::PolicyDown)]
#[tokio::test]
async fn a_group_that_does_not_load_is_dropped_but_stays_listed(#[case] failure: Stored) {
    let warnings = Warnings::default();
    let _guard = warnings.install();
    let access = groups(
        true,
        &["g1", "gone"],
        &[two_groups().remove(0), ("gone", failure)],
    );
    let ceiling = access
        .resolve_agent_ceiling("agent-1", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ceiling.access_group_ids, strings(&["g1", "gone"]));
    assert_eq!(ceiling.models, set(&["claude", "gpt-4o"]));
    assert_eq!(ceiling.mcp_server_ids, set(&["srv-b"]));
    assert_eq!(ceiling.agent_ids, set(&["agent-x"]));
    let logged = warnings.messages();
    assert_eq!(logged.len(), 1);
    assert!(
        logged[0]
            .starts_with("Agent access group gone could not be loaded, treating it as empty: ")
    );
}

#[tokio::test]
async fn an_agent_whose_only_group_vanished_gets_empty_sets_that_deny() {
    let access = groups(true, &["gone"], &[("gone", Stored::Missing)]);
    let ceiling = access
        .resolve_agent_ceiling("agent-1", false)
        .await
        .unwrap();
    let expected = AgentAccessGroupCeiling {
        access_group_ids: strings(&["gone"]),
        models: BTreeSet::new(),
        mcp_server_ids: BTreeSet::new(),
        agent_ids: BTreeSet::new(),
    };
    assert_eq!(ceiling, Some(expected));
}

#[rstest]
#[case::lenient(false)]
#[case::writer(true)]
#[tokio::test]
async fn without_a_database_every_group_is_dropped_with_a_warning(#[case] check_db_only: bool) {
    let warnings = Warnings::default();
    let _guard = warnings.install();
    let access = groups(false, &["g1", "g2"], &two_groups());
    let ceiling = access
        .resolve_agent_ceiling("agent-1", check_db_only)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ceiling.access_group_ids, strings(&["g1", "g2"]));
    assert!(ceiling.models.is_empty());
    assert!(ceiling.mcp_server_ids.is_empty());
    assert!(ceiling.agent_ids.is_empty());
    assert!(reads(&access).is_empty());
    assert_eq!(
        warnings.messages(),
        [
            "Agent access group g1 cannot be loaded without a DB",
            "Agent access group g2 cannot be loaded without a DB",
        ]
    );
}

#[tokio::test]
async fn the_writer_path_reads_every_group_with_check_db_only() {
    let access = groups(true, &["g1", "g2"], &two_groups());
    let ceiling = access
        .resolve_agent_ceiling("agent-1", true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ceiling.models, set(&["claude", "gemini", "gpt-4o"]));
    assert_eq!(
        reads(&access),
        [("g1".to_owned(), true), ("g2".to_owned(), true)]
    );
}

#[rstest]
#[case::row_missing(Stored::Missing, 404)]
#[case::policy_unavailable(Stored::PolicyDown, 503)]
#[tokio::test]
async fn the_writer_path_returns_a_failed_read(#[case] failure: Stored, #[case] status: u16) {
    let warnings = Warnings::default();
    let _guard = warnings.install();
    let access = groups(true, &["gone"], &[("gone", failure)]);
    let error = access
        .resolve_agent_ceiling("agent-1", true)
        .await
        .unwrap_err();
    let AccessGroupsError::Read {
        access_group_id,
        source,
    } = error;
    assert_eq!(access_group_id, "gone");
    assert_eq!(source.http().map(|(code, _)| code), Some(status));
    assert!(warnings.messages().is_empty());
}

#[tokio::test]
async fn a_failure_that_is_not_an_http_error_is_returned_even_when_lenient() {
    let access = groups(
        true,
        &["g1", "broken"],
        &[two_groups().remove(0), ("broken", Stored::Plain)],
    );
    let error = access
        .resolve_agent_ceiling("agent-1", false)
        .await
        .unwrap_err();
    let AccessGroupsError::Read {
        access_group_id,
        source,
    } = error;
    assert_eq!(access_group_id, "broken");
    assert!(matches!(*source, TeamLookupError::NoDatabase));
}

#[tokio::test]
async fn server_ids_are_the_sorted_lenient_union() {
    let access = groups(
        true,
        &["g1", "gone", "g2"],
        &[
            two_groups().remove(0),
            two_groups().remove(1),
            ("gone", Stored::Missing),
        ],
    );
    let servers = access
        .agent_access_group_server_ids("agent-1")
        .await
        .unwrap();
    assert_eq!(servers, Some(strings(&["srv-a", "srv-b"])));
    assert!(
        reads(&access)
            .iter()
            .all(|(_, check_db_only)| !check_db_only)
    );
}

#[tokio::test]
async fn server_ids_are_none_without_an_attached_group() {
    let access = groups(true, &[], &two_groups());
    let servers = access
        .agent_access_group_server_ids("agent-1")
        .await
        .unwrap();
    assert_eq!(servers, None);
}

#[tokio::test]
async fn server_ids_are_empty_when_every_group_vanished() {
    let access = groups(true, &["gone"], &[("gone", Stored::Missing)]);
    let servers = access
        .agent_access_group_server_ids("agent-1")
        .await
        .unwrap();
    assert_eq!(servers, Some(Vec::new()));
}

#[tokio::test]
async fn managed_ceilings_are_empty_for_no_ids_and_read_nothing() {
    let access = groups(true, &["g1"], &two_groups());
    let ceilings = access.resolve_managed_agent_ceilings(&[]).await.unwrap();
    assert!(ceilings.is_empty());
    assert!(reads(&access).is_empty());
}

#[tokio::test]
async fn managed_ceilings_read_the_given_ids_on_the_writer_and_ignore_the_registry() {
    let access = groups(true, &["g1"], &two_groups());
    let ceilings = access
        .resolve_managed_agent_ceilings(&strings(&["g2", "g2"]))
        .await
        .unwrap();
    let expected = AgentAccessGroupCeiling {
        access_group_ids: strings(&["g2", "g2"]),
        models: set(&["claude", "gemini"]),
        mcp_server_ids: set(&["srv-a"]),
        agent_ids: set(&["agent-y"]),
    };
    assert_eq!(ceilings, [expected]);
    assert_eq!(
        reads(&access),
        [("g2".to_owned(), true), ("g2".to_owned(), true)]
    );
}

#[tokio::test]
async fn managed_ceilings_return_a_failed_read() {
    let access = groups(
        true,
        &[],
        &[two_groups().remove(0), ("gone", Stored::Missing)],
    );
    let error = access
        .resolve_managed_agent_ceilings(&strings(&["g1", "gone"]))
        .await
        .unwrap_err();
    let AccessGroupsError::Read {
        access_group_id, ..
    } = error;
    assert_eq!(access_group_id, "gone");
}

#[tokio::test]
async fn managed_ceilings_without_a_database_keep_the_ids_with_empty_sets() {
    let access = groups(false, &[], &two_groups());
    let ceilings = access
        .resolve_managed_agent_ceilings(&strings(&["g1"]))
        .await
        .unwrap();
    let expected = AgentAccessGroupCeiling {
        access_group_ids: strings(&["g1"]),
        models: BTreeSet::new(),
        mcp_server_ids: BTreeSet::new(),
        agent_ids: BTreeSet::new(),
    };
    assert_eq!(ceilings, [expected]);
}
