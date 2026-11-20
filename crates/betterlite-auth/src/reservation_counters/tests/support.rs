use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::{self, PayloadError};
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use crate::auth_tag_budgets::groups::ModelAccessGroupBudget;
use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure};
use crate::budget_counter::BudgetCounter;
use crate::budget_primitives::window_start::DurationParser;
use crate::reservation_counters::cache::ReservationCounterCache;
use crate::reservation_counters::error::ReservationCounterError;
use crate::reservation_counters::{CounterInput, CounterServices, budget_counters};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(super) const NOW_SECONDS: i64 = 1_704_067_200;
pub(super) const TTL: f64 = 60.0;

pub(super) fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(NOW_SECONDS, 0).expect("the fixed time")
}

pub(super) fn epoch_seconds(moment: SystemTime) -> i64 {
    let after = moment
        .duration_since(UNIX_EPOCH)
        .expect("the moment lies after the unix epoch");
    i64::try_from(after.as_secs()).expect("the seconds fit in i64")
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid json")
}

pub(super) fn decoded_row<Row>(text: &str) -> Row
where
    Row: DeserializeOwned,
{
    serde_json::from_str(text).expect("the row fixture decodes")
}

pub(super) fn auth(text: &str) -> UserAPIKeyAuth {
    let JsonData::Object(fields) = decoded_row(text) else {
        panic!("the auth fixture is a json object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the auth fixture builds")
}

pub(super) fn payload_error() -> PayloadError {
    PayloadError::Json(serde_json::from_str::<i64>("not a number").expect_err("the invalid json"))
}

pub(super) fn only(counters: &[BudgetCounter]) -> &BudgetCounter {
    assert_eq!(counters.len(), 1, "the fixture builds exactly one counter");
    &counters[0]
}

pub(super) fn dec(text: &str) -> Decimal {
    text.parse().expect("the decimal fixture")
}

pub(super) struct FixedDurations {
    calls: Mutex<Vec<(String, DateTime<Utc>)>>,
}

impl FixedDurations {
    pub(super) fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<(String, DateTime<Utc>)> {
        self.calls.lock().expect("the duration calls lock").clone()
    }

    pub(super) fn specs(&self) -> Vec<String> {
        let calls = self.calls();
        calls.into_iter().map(|(spec, _now)| spec).collect()
    }
}

impl DurationParser for FixedDurations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        self.calls
            .lock()
            .expect("the duration calls lock")
            .push((spec.to_owned(), now));
        match spec {
            "1d" => Some(86_400),
            "1h" => Some(3_600),
            _ => None,
        }
    }
}

pub(super) struct FakeCache {
    values: Mutex<BTreeMap<String, Json>>,
    reads: Mutex<Vec<String>>,
    broken: bool,
}

impl FakeCache {
    pub(super) fn new() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            broken: false,
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            broken: true,
            ..Self::new()
        }
    }

    pub(super) fn put<Value>(&self, key: &str, value: &Value)
    where
        Value: Serialize,
    {
        let encoded = payload::encode(value).expect("the cached fixture encodes");
        self.put_encoded(key, encoded);
    }

    pub(super) fn put_encoded(&self, key: &str, value: Json) {
        self.values
            .lock()
            .expect("the cache values lock")
            .insert(key.to_owned(), value);
    }

    pub(super) fn read_keys(&self) -> Vec<String> {
        self.reads.lock().expect("the cache reads lock").clone()
    }

    fn probe<Row>(&self, key: &str) -> Result<Option<Row>, PayloadError>
    where
        Row: DeserializeOwned,
    {
        self.reads
            .lock()
            .expect("the cache reads lock")
            .push(key.to_owned());
        if self.broken {
            return Err(payload_error());
        }
        let stored = self
            .values
            .lock()
            .expect("the cache values lock")
            .get(key)
            .cloned();
        stored.map_or(Ok(None), |value| payload::decode(&value))
    }
}

#[async_trait]
impl TagBudgetCache for FakeCache {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        self.probe(key)
    }

    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        self.probe(key)
    }

    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(self
            .values
            .lock()
            .expect("the cache values lock")
            .get(key)
            .cloned())
    }

    async fn write(
        &self,
        key: &str,
        value: CachedValue<'_>,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        let encoded = match value {
            CachedValue::Tag(row) => payload::encode(row),
            CachedValue::Group(row) => payload::encode(row),
            CachedValue::Registry(answer) => Ok(answer.clone()),
        }?;
        self.put_encoded(key, encoded);
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        TTL
    }
}

#[async_trait]
impl ReservationCounterCache for FakeCache {
    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, PayloadError> {
        self.probe(key)
    }

    async fn read_organization(
        &self,
        key: &str,
    ) -> Result<Option<OrganizationTable>, PayloadError> {
        self.probe(key)
    }

    async fn read_membership(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, PayloadError> {
        self.probe(key)
    }

    async fn read_default_budget(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, PayloadError> {
        self.probe(key)
    }
}

#[derive(Default)]
pub(super) struct FakeRows {
    tags: Vec<TagTable>,
    groups: Vec<ModelAccessGroupBudgetRow>,
    tag_queries: Mutex<Vec<Vec<String>>>,
    group_queries: Mutex<Vec<Vec<String>>>,
}

impl FakeRows {
    pub(super) fn with_tags(self, tags: Vec<TagTable>) -> Self {
        Self { tags, ..self }
    }

    pub(super) fn with_groups(self, groups: Vec<ModelAccessGroupBudgetRow>) -> Self {
        Self { groups, ..self }
    }

    pub(super) fn tag_queries(&self) -> Vec<Vec<String>> {
        self.tag_queries
            .lock()
            .expect("the tag queries lock")
            .clone()
    }

    pub(super) fn group_queries(&self) -> Vec<Vec<String>> {
        self.group_queries
            .lock()
            .expect("the group queries lock")
            .clone()
    }
}

fn matching<Row>(rows: &[Row], names: &[String], name: fn(&Row) -> &str) -> Vec<Row>
where
    Row: Clone,
{
    rows.iter()
        .filter(|row| names.iter().any(|wanted| wanted.as_str() == name(row)))
        .cloned()
        .collect()
}

#[async_trait]
impl BudgetRows<TagTable> for FakeRows {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        self.tag_queries
            .lock()
            .expect("the tag queries lock")
            .push(names.clone());
        Ok(matching(&self.tags, &names, |row| &row.tag_name))
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self.tags.clone())
    }
}

#[async_trait]
impl BudgetRows<ModelAccessGroupBudgetRow> for FakeRows {
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        self.group_queries
            .lock()
            .expect("the group queries lock")
            .push(names.clone());
        Ok(matching(&self.groups, &names, |row| &row.access_group_name))
    }

    async fn registry_rows(
        &self,
        _take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(self.groups.clone())
    }
}

pub(super) struct Fixture {
    pub(super) token: UserAPIKeyAuth,
    pub(super) team: Option<LiteLLMTeamTable>,
    pub(super) user: Option<LiteLLMUserTable>,
    pub(super) end_user_id: Option<String>,
    pub(super) end_user: Option<LiteLLMEndUserTable>,
    pub(super) tag_names: Vec<String>,
    pub(super) apply_override: bool,
    pub(super) database: bool,
    pub(super) cache: FakeCache,
    pub(super) rows: FakeRows,
    pub(super) durations: FixedDurations,
    pub(super) now: DateTime<Utc>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            token: auth(r#"{"token":"tok"}"#),
            team: None,
            user: None,
            end_user_id: None,
            end_user: None,
            tag_names: Vec::new(),
            apply_override: false,
            database: true,
            cache: FakeCache::new(),
            rows: FakeRows::default(),
            durations: FixedDurations::new(),
            now: now(),
        }
    }

    pub(super) async fn run(&self) -> Result<Vec<BudgetCounter>, ReservationCounterError> {
        let input = CounterInput {
            token: &self.token,
            team: self.team.as_ref(),
            user: self.user.as_ref(),
            end_user_id: self.end_user_id.as_deref(),
            end_user: self.end_user.as_ref(),
            tag_names: &self.tag_names,
            apply_user_budget_to_team_keys: self.apply_override,
        };
        let services = CounterServices {
            database: self.database,
            cache: &self.cache,
            tags: &self.rows,
            groups: &self.rows,
            durations: &self.durations,
            now: self.now,
        };
        budget_counters(&input, &services).await
    }

    pub(super) async fn counters(&self) -> Vec<BudgetCounter> {
        self.run().await.expect("the counter list builds")
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(super) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(logs.clone())
            .finish();
        (logs, set_default(subscriber))
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("the captured logs lock").clone())
            .expect("the captured logs are UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("the captured logs lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn tag_row(name: &str, spend: &str, budget: &str) -> TagTable {
    decoded_row(&format!(
        r#"{{"tag_name":"{name}","models":[],"spend":{spend},"litellm_budget_table":{budget}}}"#
    ))
}

pub(super) fn group_row(name: &str, spend: &str, budget: &str) -> ModelAccessGroupBudgetRow {
    decoded_row(&format!(
        r#"{{"access_group_name":"{name}","spend":{spend},"litellm_budget_table":{budget}}}"#
    ))
}

pub(super) fn group_budget(name: &str, spend: &str, max_budget: &str) -> ModelAccessGroupBudget {
    decoded_row(&format!(
        r#"{{"access_group_name":"{name}","spend":{spend},"max_budget":{max_budget}}}"#
    ))
}

pub(super) fn membership_row(budget: &str, spend: &str) -> LiteLLMTeamMembership {
    decoded_row(&format!(
        r#"{{"user_id":"user-a","team_id":"team-a","spend":{spend},"litellm_budget_table":{budget}}}"#
    ))
}

pub(super) fn org_row(budget: &str, spend: &str) -> OrganizationTable {
    decoded_row(&format!(
        r#"{{"organization_id":"org-a","budget_id":"budget-a","spend":{spend},"created_by":"admin","updated_by":"admin","litellm_budget_table":{budget}}}"#
    ))
}

pub(super) fn project_row(budget: &str, spend: &str) -> CachedProject {
    decoded_row(&format!(
        r#"{{"project_id":"project-a","spend":{spend},"litellm_budget_table":{budget},"last_refreshed_at":null}}"#
    ))
}

pub(super) fn member_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","user_id":"user-a"}"#);
    fixture.team = Some(decoded_row(r#"{"team_id":"team-a"}"#));
    fixture.user = Some(decoded_row(r#"{"user_id":"user-a"}"#));
    fixture
}

pub(super) fn full_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.apply_override = true;
    fixture.token = auth(concat!(
        r#"{"token":"tok","max_budget":100,"spend":1,"user_id":"user-a","end_user_id":"eu-1","#,
        r#""end_user_max_budget":50,"project_id":"project-a","org_id":"org-a","#,
        r#""matched_model_access_groups":["group-a"],"#,
        r#""budget_limits":[{"budget_duration":"1d","max_budget":5}]}"#,
    ));
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","max_budget":90,"spend":2}"#,
    ));
    fixture.user = Some(decoded_row(
        r#"{"user_id":"user-a","max_budget":80,"spend":3}"#,
    ));
    fixture.end_user = Some(decoded_row(
        r#"{"user_id":"eu-1","blocked":false,"spend":4,"litellm_budget_table":{"max_budget":40}}"#,
    ));
    fixture.tag_names = vec!["tag-a".to_owned()];
    fixture.rows = FakeRows::default()
        .with_tags(vec![tag_row("tag-a", "9", r#"{"max_budget":30}"#)])
        .with_groups(vec![group_row("group-a", "8", r#"{"max_budget":20}"#)]);
    fixture.cache.put(
        "team_membership:user-a:team-a",
        &membership_row(r#"{"max_budget":10}"#, "5"),
    );
    fixture.cache.put(
        "org_id:org-a:with_budget",
        &org_row(r#"{"max_budget":15}"#, "6"),
    );
    fixture.cache.put(
        "project_id:project-a",
        &project_row(r#"{"max_budget":12}"#, "7"),
    );
    fixture
}
