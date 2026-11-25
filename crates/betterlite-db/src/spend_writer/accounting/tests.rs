#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::daily_spend_sql::values::SpendColumnValue;
use crate::model_usage::ModelUsageTransaction;
use crate::redis_spend::codec::decode_log_record;
use crate::redis_spend::log_row::SpendLogRow;
use crate::request_usage::queues::UsageQueue;
use crate::spend_queues::entity::{EntityType, SpendAmount, SpendUpdateQueueItem};
use crate::spend_writer::claim::ScriptedClaim;
use crate::spend_writer::settings::SpendWriterSettings;
use crate::spend_writer::support::{
    Case, RecordedFailure, RecordingAdmission, RecordingDailyQueue, RecordingEntityQueue,
    RecordingFailures,
};
use crate::spend_writer::{DbSpendUpdateWriter, SpendWriterOperation, SpendWriterQueues};

use super::{SpendAccounting, SpendAccountingEnvironment, batch_database_updates};

const REQUEST: &str = r#"{"user":"u1","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","prompt_tokens":3,"completion_tokens":4,"request_id":"req-1","startTime":"2025-01-02T03:04:05Z","endTime":"2025-01-02T03:04:06Z","request_tags":"[\"tag-a\",\"tag-b\"]","metadata":"{}","status":"success","spend":0.25,"team_id":"team-1","organization_id":"org-1","end_user":"eu-1","agent_id":"agent-1"}"#;
const SUFFIX: &str = "_2025-01-02_k1_gpt-4o_openai_";

fn cost() -> SpendAmount {
    SpendAmount(Decimal::new(25, 1))
}

fn row(text: &str) -> SpendLogRow {
    decode_log_record(text.as_bytes())
        .expect("the test payload decodes")
        .expect("the test payload is a JSON object")
}

fn work_for(payload: SpendLogRow) -> SpendAccounting {
    SpendAccounting {
        response_cost: Some(cost()),
        user_id: Some("u1".to_owned()),
        hashed_token: Some("hash-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        org_id: Some("org-1".to_owned()),
        project_id: Some("proj-1".to_owned()),
        end_user_id: Some("eu-1".to_owned()),
        model_access_groups: vec!["g1".to_owned(), "g2".to_owned()],
        payload,
    }
}

fn ids(items: &[SpendUpdateQueueItem]) -> Vec<(EntityType, String)> {
    items
        .iter()
        .map(|item| {
            (
                item.entity_type.clone().expect("every charge names a kind"),
                item.entity_id.clone().expect("every charge names an id"),
            )
        })
        .collect()
}

fn daily_keys(queue: &RecordingDailyQueue) -> Vec<String> {
    queue
        .updates()
        .into_iter()
        .flat_map(|update| update.into_keys())
        .collect()
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the captured log is UTF-8")
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buffer: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(logs.clone())
        .finish();
    (logs, set_default(subscriber))
}

struct Harness {
    entity: RecordingEntityQueue,
    daily_user: RecordingDailyQueue,
    daily_end_user: RecordingDailyQueue,
    daily_agent: RecordingDailyQueue,
    daily_team: RecordingDailyQueue,
    daily_org: RecordingDailyQueue,
    daily_tag: RecordingDailyQueue,
    model_usage: UsageQueue<ModelUsageTransaction>,
    failures: RecordingFailures,
    admission: RecordingAdmission,
}

impl Harness {
    fn new() -> Self {
        Self::with_failures(None, None)
    }

    fn with_failures(entity_fail_at: Option<usize>, daily_user_fail_at: Option<usize>) -> Self {
        Self {
            entity: RecordingEntityQueue::new(entity_fail_at),
            daily_user: RecordingDailyQueue::new(daily_user_fail_at),
            daily_end_user: RecordingDailyQueue::new(None),
            daily_agent: RecordingDailyQueue::new(None),
            daily_team: RecordingDailyQueue::new(None),
            daily_org: RecordingDailyQueue::new(None),
            daily_tag: RecordingDailyQueue::new(None),
            model_usage: UsageQueue::new(100).expect("the model usage queue"),
            failures: RecordingFailures::new(),
            admission: RecordingAdmission::admitting(),
        }
    }

    fn writer(&self, settings: SpendWriterSettings) -> DbSpendUpdateWriter<'_, ScriptedClaim> {
        DbSpendUpdateWriter {
            settings,
            queues: SpendWriterQueues {
                entity: Some(&self.entity),
                daily_user: Some(&self.daily_user),
                daily_end_user: Some(&self.daily_end_user),
                daily_agent: Some(&self.daily_agent),
                daily_team: Some(&self.daily_team),
                daily_org: Some(&self.daily_org),
                daily_tag: Some(&self.daily_tag),
                tool_usage: None,
                model_usage: Some(&self.model_usage),
                discovery: None,
                turns: None,
            },
            attachment: None,
            failures: &self.failures,
            admission: &self.admission,
        }
    }

    async fn run(&self, charge: &SpendAccounting, settings: SpendWriterSettings) {
        let case = Case::new();
        let pricing = case.pricing();
        let environment = SpendAccountingEnvironment {
            pricing: &pricing,
            endpoints: &case.endpoints,
            deployments: None,
        };
        let writer = self.writer(settings);
        batch_database_updates(&writer, charge, &environment).await;
    }
}

#[tokio::test]
async fn charges_every_entity_and_daily_row_in_source_order() {
    let harness = Harness::new();
    let charge = work_for(row(REQUEST));
    harness.run(&charge, SpendWriterSettings::default()).await;

    assert_eq!(
        ids(&harness.entity.items()),
        vec![
            (EntityType::User, "u1".to_owned()),
            (EntityType::EndUser, "eu-1".to_owned()),
            (EntityType::Key, "hash-1".to_owned()),
            (EntityType::Team, "team-1".to_owned()),
            (
                EntityType::TeamMember,
                "team_id::team-1::user_id::u1".to_owned()
            ),
            (EntityType::Organization, "org-1".to_owned()),
            (
                EntityType::OrganizationMember,
                "organization_id::org-1::user_id::u1".to_owned()
            ),
            (EntityType::Project, "proj-1".to_owned()),
            (EntityType::Tag, "tag-a".to_owned()),
            (EntityType::Tag, "tag-b".to_owned()),
            (EntityType::ModelAccessGroup, "g1".to_owned()),
            (EntityType::ModelAccessGroup, "g2".to_owned()),
            (EntityType::Agent, "agent-1".to_owned()),
        ]
    );
    assert!(
        harness
            .entity
            .items()
            .iter()
            .all(|item| item.response_cost == Some(cost()))
    );
    assert_eq!(daily_keys(&harness.daily_user), vec![format!("u1{SUFFIX}")]);
    assert_eq!(
        daily_keys(&harness.daily_end_user),
        vec![format!("eu-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_agent),
        vec![format!("agent-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_team),
        vec![format!("team-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_org),
        vec![format!("org-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_tag),
        vec![format!("tag-a{SUFFIX}"), format!("tag-b{SUFFIX}")]
    );
    let usage = harness.model_usage.drain(usize::MAX);
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].key.date, "2025-01-02");
    assert_eq!(usage[0].key.task_type, "uncategorized");
    assert_eq!(usage[0].spend, cost());
    assert!(harness.failures.reported().is_empty());
}

#[tokio::test]
async fn a_failed_user_helper_is_reported_and_the_later_entities_charge() {
    let harness = Harness::with_failures(Some(0), None);
    let charge = work_for(row(REQUEST));
    harness.run(&charge, SpendWriterSettings::default()).await;
    assert_eq!(
        harness.failures.reported(),
        vec![RecordedFailure {
            operation: SpendWriterOperation::UserEnqueue,
            token: Some("hash-1".to_owned()),
        }]
    );
    let charged = ids(&harness.entity.items());
    assert!(!charged.iter().any(|(kind, _)| kind == &EntityType::User));
    assert!(!charged.iter().any(|(kind, _)| kind == &EntityType::EndUser));
    assert!(charged.contains(&(EntityType::Key, "hash-1".to_owned())));
    assert!(charged.contains(&(EntityType::Team, "team-1".to_owned())));
    assert!(charged.contains(&(EntityType::Agent, "agent-1".to_owned())));
}

#[tokio::test]
async fn a_failed_key_enqueue_is_reported_and_the_later_entities_charge() {
    let harness = Harness::with_failures(Some(0), None);
    let mut charge = work_for(row(REQUEST));
    charge.user_id = None;
    charge.end_user_id = None;
    harness.run(&charge, SpendWriterSettings::default()).await;
    assert_eq!(
        harness.failures.reported(),
        vec![RecordedFailure {
            operation: SpendWriterOperation::KeyEnqueue,
            token: Some("hash-1".to_owned()),
        }]
    );
    let charged = ids(&harness.entity.items());
    assert!(!charged.iter().any(|(kind, _)| kind == &EntityType::Key));
    assert!(charged.contains(&(EntityType::Team, "team-1".to_owned())));
    assert!(charged.contains(&(EntityType::Agent, "agent-1".to_owned())));
}

#[tokio::test]
async fn a_failed_daily_helper_leaves_the_other_daily_rows_and_model_usage_running() {
    let harness = Harness::with_failures(None, Some(0));
    let charge = work_for(row(REQUEST));
    harness.run(&charge, SpendWriterSettings::default()).await;
    assert!(daily_keys(&harness.daily_user).is_empty());
    assert_eq!(
        daily_keys(&harness.daily_end_user),
        vec![format!("eu-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_agent),
        vec![format!("agent-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_team),
        vec![format!("team-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_org),
        vec![format!("org-1{SUFFIX}")]
    );
    assert_eq!(
        daily_keys(&harness.daily_tag),
        vec![format!("tag-a{SUFFIX}"), format!("tag-b{SUFFIX}")]
    );
    assert_eq!(harness.model_usage.len(), 1);
    assert!(harness.failures.reported().is_empty());
}

#[tokio::test]
async fn a_failed_daily_helper_is_logged_and_the_later_rows_still_run() {
    let harness = Harness::with_failures(None, Some(0));
    let charge = work_for(row(REQUEST));
    let (logs, _captured) = capture_logs();
    harness.run(&charge, SpendWriterSettings::default()).await;
    let logged = logs.contents();
    assert!(logged.contains("daily user"), "{logged}");
    assert!(
        logged.contains("_batch_database_updates: the helper failed"),
        "{logged}"
    );
    assert_eq!(
        daily_keys(&harness.daily_tag),
        vec![format!("tag-a{SUFFIX}"), format!("tag-b{SUFFIX}")]
    );
}

#[tokio::test]
async fn charges_the_proxy_identity_only_with_a_positive_budget_and_never_dedupes_it() {
    let harness = Harness::new();
    let charge = work_for(row(REQUEST));
    let settings = SpendWriterSettings {
        disable_spend_updates: false,
        disable_spend_logs: Some(false),
        max_budget: Decimal::from(10_i64),
        proxy_budget_name: "litellm-proxy-budget".to_owned(),
    };
    harness.run(&charge, settings).await;
    let charged = ids(&harness.entity.items());
    assert_eq!(
        charged[..3].to_vec(),
        vec![
            (EntityType::User, "u1".to_owned()),
            (EntityType::User, "litellm-proxy-budget".to_owned()),
            (EntityType::EndUser, "eu-1".to_owned()),
        ]
    );

    let duplicate = Harness::new();
    let mut duplicate_charge = work_for(row(REQUEST));
    duplicate_charge.user_id = Some("litellm-proxy-budget".to_owned());
    duplicate_charge.end_user_id = None;
    let duplicate_settings = SpendWriterSettings {
        disable_spend_updates: false,
        disable_spend_logs: Some(false),
        max_budget: Decimal::ONE,
        proxy_budget_name: "litellm-proxy-budget".to_owned(),
    };
    duplicate.run(&duplicate_charge, duplicate_settings).await;
    assert_eq!(
        ids(&duplicate.entity.items()),
        vec![
            (EntityType::User, "litellm-proxy-budget".to_owned()),
            (EntityType::User, "litellm-proxy-budget".to_owned()),
        ]
    );
}

#[tokio::test]
async fn charges_daily_organization_under_the_request_s_own_id() {
    let harness = Harness::new();
    let mut charge = work_for(row(REQUEST));
    charge.org_id = Some("work-org".to_owned());
    harness.run(&charge, SpendWriterSettings::default()).await;
    assert!(
        ids(&harness.entity.items()).contains(&(EntityType::Organization, "work-org".to_owned()))
    );
    let updates = harness.daily_org.updates();
    assert_eq!(
        updates
            .iter()
            .flat_map(|update| update.keys())
            .cloned()
            .collect::<Vec<String>>(),
        vec![format!("work-org{SUFFIX}")]
    );
    let organization = updates[0]
        .values()
        .next()
        .expect("the daily organization row")
        .get("organization_id");
    assert_eq!(
        organization,
        Some(&SpendColumnValue::Text("work-org".to_owned()))
    );
}

#[tokio::test]
async fn charges_entities_without_a_daily_projection() {
    let harness = Harness::new();
    let charge = work_for(row(
        r#"{"user":"u1","model":"gpt-4o","startTime":"2025-01-02T03:04:05Z","spend":0.25}"#,
    ));
    harness.run(&charge, SpendWriterSettings::default()).await;
    let charged = ids(&harness.entity.items());
    assert!(charged.contains(&(EntityType::User, "u1".to_owned())));
    assert!(charged.contains(&(EntityType::Key, "hash-1".to_owned())));
    assert!(charged.contains(&(EntityType::Team, "team-1".to_owned())));
    assert!(!charged.iter().any(|(kind, _)| kind == &EntityType::Tag));
    assert!(!charged.iter().any(|(kind, _)| kind == &EntityType::Agent));
    assert!(daily_keys(&harness.daily_user).is_empty());
    assert!(daily_keys(&harness.daily_org).is_empty());
    assert!(harness.model_usage.is_empty());
}

#[tokio::test]
async fn reads_an_already_parsed_tag_list_without_deduping_it() {
    let harness = Harness::new();
    let charge = work_for(row(
        r#"{"user":"u1","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","request_id":"req-2","startTime":"2025-01-02T03:04:05Z","request_tags":["tag-a","tag-a","tag-b",""],"metadata":"{}","status":"success","spend":0.25}"#,
    ));
    harness.run(&charge, SpendWriterSettings::default()).await;
    let tags: Vec<String> = ids(&harness.entity.items())
        .into_iter()
        .filter(|(kind, _)| kind == &EntityType::Tag)
        .map(|(_, tag)| tag)
        .collect();
    assert_eq!(tags, vec!["tag-a", "tag-a", "tag-b"]);
    assert_eq!(
        daily_keys(&harness.daily_tag),
        vec![format!("tag-a{SUFFIX}"), format!("tag-b{SUFFIX}")]
    );
}
