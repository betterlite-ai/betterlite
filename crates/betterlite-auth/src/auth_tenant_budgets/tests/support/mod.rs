use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;
use crate::auth_key_budgets::money::Money;
use crate::auth_key_budgets::spend::{SpendReadError, SpendReader};
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_org_lookup::ports::{OrgCache, OrgStore};
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use crate::auth_tag_budgets::groups::ModelAccessGroupBudget;
use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure};
use crate::auth_tenant_budgets::TenantSources;
use crate::auth_tenant_budgets::alerts::{TenantAlert, TenantAlertSink};
use crate::current_spend::SpendQuery;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

mod org_world;

pub(crate) use org_world::OrgWorld;

// the query one spend read carried, as the counter sees it
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpendRead {
    pub(crate) counter_key: String,
    pub(crate) fallback_spend: Decimal,
    pub(crate) max_budget: Option<Decimal>,
    pub(crate) fallback_authoritative: bool,
}

// the counter values a case scripts, handed out in read order, and the query each read carried
pub(crate) struct ScriptedSpend {
    spends: Vec<JsonData>,
    fail_at: Option<usize>,
    reads: Mutex<Vec<SpendRead>>,
}

impl ScriptedSpend {
    pub(crate) fn new(spends: Vec<JsonData>) -> Self {
        Self {
            spends,
            fail_at: None,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing(spends: Vec<JsonData>, at: usize) -> Self {
        Self {
            spends,
            fail_at: Some(at),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<SpendRead> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl SpendReader for ScriptedSpend {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        let index = {
            let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
            reads.push(SpendRead {
                counter_key: query.counter_key.to_owned(),
                fallback_spend: query.fallback_spend,
                max_budget: query.max_budget,
                fallback_authoritative: query.fallback_authoritative,
            });
            reads.len() - 1
        };
        if Some(index) == self.fail_at {
            return Err(SpendReadError::Unverifiable {
                counter_key: query.counter_key.to_owned(),
            });
        }
        self.spends
            .get(index)
            .and_then(Money::from_json)
            .and_then(Money::finite_value)
            .ok_or_else(|| SpendReadError::Unverifiable {
                counter_key: query.counter_key.to_owned(),
            })
    }
}

// records what each check hands its sink; TenantAlert carries no PartialEq, so tests read the fields
#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) alerts: Mutex<Vec<TenantAlert>>,
}

impl Recorder {
    pub(crate) fn take(&self) -> Vec<TenantAlert> {
        std::mem::take(&mut *self.alerts.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl TenantAlertSink for Recorder {
    fn schedule(&self, alert: TenantAlert) {
        self.alerts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(alert);
    }
}

// the B740 collaborators over in-memory rows; a missing name never reaches the database, like the real tables
#[derive(Default)]
pub(crate) struct TagWorld {
    pub(crate) tags: BTreeMap<String, TagTable>,
    pub(crate) groups: BTreeMap<String, ModelAccessGroupBudgetRow>,
    pub(crate) tag_batches: Mutex<usize>,
    pub(crate) group_batches: Mutex<usize>,
}

impl TagWorld {
    pub(crate) fn new(tags: Vec<TagTable>, groups: Vec<ModelAccessGroupBudgetRow>) -> Self {
        Self {
            tags: tags
                .into_iter()
                .map(|tag| (tag.tag_name.clone(), tag))
                .collect(),
            groups: groups
                .into_iter()
                .map(|group| (group.access_group_name.clone(), group))
                .collect(),
            tag_batches: Mutex::new(0),
            group_batches: Mutex::new(0),
        }
    }

    pub(crate) fn batches(&self) -> (usize, usize) {
        let tags = *self
            .tag_batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let groups = *self
            .group_batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (tags, groups)
    }
}

#[async_trait]
impl TagBudgetCache for TagWorld {
    async fn read_tag(&self, _key: &str) -> Result<Option<TagTable>, PayloadError> {
        Ok(None)
    }

    async fn read_group(&self, _key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        Ok(None)
    }

    async fn read_registry(&self, _key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _value: CachedValue<'_>,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl BudgetRows<TagTable> for TagWorld {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        *self
            .tag_batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
        Ok(self
            .tags
            .values()
            .filter(|tag| names.contains(&tag.tag_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self.tags.values().cloned().collect())
    }
}

#[async_trait]
impl BudgetRows<ModelAccessGroupBudgetRow> for TagWorld {
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        *self
            .group_batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
        let mut found = Vec::new();
        for name in names {
            if let Some(group) = self.groups.get(&name) {
                found.push(group.clone());
            }
        }
        Ok(found)
    }

    async fn registry_rows(
        &self,
        _take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(self.groups.values().cloned().collect())
    }
}

pub(crate) fn tenant_sources(world: &TagWorld, database: bool) -> TenantSources<'_> {
    let cache: &dyn TagBudgetCache = world;
    let tags: &dyn BudgetRows<TagTable> = world;
    let groups: &dyn BudgetRows<ModelAccessGroupBudgetRow> = world;
    TenantSources {
        database,
        organizations: None,
        cache: Some(cache),
        tags: Some(tags),
        groups: Some(groups),
    }
}

pub(crate) fn lookup(world: &Arc<OrgWorld>) -> OrgLookup {
    let store: Arc<dyn OrgStore> = world.clone();
    let cache: Arc<dyn OrgCache> = world.clone();
    OrgLookup {
        store: Some(store),
        cache,
        in_memory_ttl: 60.0,
    }
}

pub(crate) fn token<T: Serialize>(fields: T) -> UserAPIKeyAuth {
    let value = serde_json::to_value(fields).expect("the token fields encode");
    let decoded: JsonData = serde_json::from_value(value).expect("the token fields decode");
    let JsonData::Object(map) = decoded else {
        panic!("the token fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the token constructs")
}

pub(crate) fn number(value: f64) -> JsonData {
    serde_json::from_value(json!(value)).expect("the counter value decodes")
}

pub(crate) fn project(spend: f64, budget: Option<(Option<f64>, Option<f64>)>) -> CachedProject {
    let mut fields = json!({"project_id": "proj-1", "spend": spend});
    if let Some((max_budget, soft_budget)) = budget {
        fields["litellm_budget_table"] =
            json!({"max_budget": max_budget, "soft_budget": soft_budget});
    }
    serde_json::from_value(fields).expect("the project row decodes")
}

// the outer option is the budget relation, the inner one its ceiling, so a row can carry a budget table without one
pub(crate) fn tag(name: &str, spend: f64, budget: Option<Option<f64>>) -> TagTable {
    let mut fields = json!({"tag_name": name, "models": [], "spend": spend});
    if let Some(max_budget) = budget {
        fields["litellm_budget_table"] = json!({"max_budget": max_budget});
    }
    serde_json::from_value(fields).expect("the tag row decodes")
}

pub(crate) fn group(
    name: &str,
    spend: f64,
    budget: Option<Option<f64>>,
) -> ModelAccessGroupBudgetRow {
    let mut fields = json!({"access_group_name": name, "spend": spend});
    if let Some(max_budget) = budget {
        fields["litellm_budget_table"] = json!({"max_budget": max_budget});
    }
    serde_json::from_value(fields).expect("the group row decodes")
}

pub(crate) fn organization(alias: &str, spend: f64, max_budget: Option<f64>) -> OrganizationTable {
    let mut fields = json!({
        "organization_id": "org-1",
        "organization_alias": alias,
        "budget_id": "budget-1",
        "spend": spend,
        "created_by": "creator",
        "updated_by": "updater",
    });
    if max_budget.is_some() {
        fields["litellm_budget_table"] = json!({"max_budget": max_budget});
    }
    serde_json::from_value(fields).expect("the organization row decodes")
}

pub(crate) fn team(organization_id: Option<&str>) -> LiteLLMTeamTable {
    serde_json::from_value(json!({"team_id": "team-1", "organization_id": organization_id}))
        .expect("the team row decodes")
}

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("the decimal spelling parses")
}

// LiteLLM's verbose_proxy_logger.debug records; the project soft check is the only one these tests read
#[derive(Clone, Default)]
pub(crate) struct EventLog(pub(crate) Arc<Mutex<Vec<(Level, String)>>>);

impl EventLog {
    pub(crate) fn messages(&self) -> Vec<(Level, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct EventFields {
    message: String,
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for EventLog {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = EventFields {
            message: String::new(),
        };
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), fields.message));
    }
}

pub(crate) fn capture_events() -> (EventLog, tracing::subscriber::DefaultGuard) {
    let log = EventLog::default();
    let guard = tracing::subscriber::set_default(registry().with(log.clone()));
    (log, guard)
}
