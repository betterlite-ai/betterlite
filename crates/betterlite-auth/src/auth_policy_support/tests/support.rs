use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::auth_cache::AuthCacheError;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use crate::auth_policy_support::extract::ToolRouteCallTypes;
use crate::auth_prefetch::rows::CachedProject;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

// a route table a test writes out, standing in for B207's catalog
#[derive(Debug, Default)]
pub(crate) struct FixedRoutes {
    call_types: BTreeMap<String, Vec<String>>,
}

impl ToolRouteCallTypes for FixedRoutes {
    fn call_types_for_route(&self, route: &str) -> Vec<String> {
        self.call_types.get(route).cloned().unwrap_or_default()
    }
}

pub(crate) fn routes(entries: &[(&str, &[&str])]) -> Arc<FixedRoutes> {
    let mut call_types = BTreeMap::new();
    for (route, types) in entries {
        call_types.insert(
            (*route).to_owned(),
            types.iter().map(|name| (*name).to_owned()).collect(),
        );
    }
    Arc::new(FixedRoutes { call_types })
}

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

pub(crate) fn body(text: &str) -> JsonObject {
    let Json::Object(fields) =
        serde_json::from_str::<Json>(text).expect("the fixture is valid JSON")
    else {
        panic!("the request body fixture must be an object");
    };
    fields
}

pub(crate) fn key(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = json(fields) else {
        panic!("the token fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the key fixture constructs")
}

pub(crate) fn team(fields: &str) -> LiteLLMTeamTable {
    serde_json::from_str(fields).expect("the team fixture decodes")
}

pub(crate) fn proxy_exception_denial() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "denied".to_owned(),
        error_type: "auth_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(403),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

// the debug lines the module logged, so a swallowed failure and a re-raised one are distinguishable
#[derive(Clone, Default)]
pub(crate) struct Logs {
    seen: Arc<Mutex<Vec<String>>>,
}

impl Logs {
    pub(crate) fn install(&self) -> DefaultGuard {
        set_default(tracing_subscriber::registry().with(self.clone()))
    }

    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        if !event
            .metadata()
            .target()
            .starts_with("betterlite_auth::auth_policy_support")
        {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{} {}", event.metadata().level(), message.0));
    }
}

// the organization cache and repository over one recorded state, so a test can tell a lookup from a skip
#[derive(Default)]
pub(crate) struct OrgWorld {
    cached: Mutex<BTreeMap<String, OrganizationTable>>,
    projects: Mutex<BTreeMap<String, String>>,
    rows: Mutex<Vec<OrganizationTable>>,
    reads: AtomicUsize,
}

impl OrgWorld {
    pub(crate) fn seed(&self, org: OrganizationTable) {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(org);
    }

    pub(crate) fn reads(&self) -> usize {
        self.reads.load(Ordering::Acquire)
    }
}

pub(crate) fn org_world() -> Arc<OrgWorld> {
    Arc::new(OrgWorld::default())
}

pub(crate) fn org_lookup(world: &Arc<OrgWorld>) -> OrgLookup {
    let cache: Arc<dyn OrgCache> = Arc::<OrgWorld>::clone(world);
    let store: Arc<dyn OrgStore> = Arc::<OrgWorld>::clone(world);
    OrgLookup {
        store: Some(store),
        cache,
        in_memory_ttl: 5.0_f64,
    }
}

pub(crate) fn org_lookup_without_database(world: &Arc<OrgWorld>) -> OrgLookup {
    let mut lookup = org_lookup(world);
    lookup.store = None;
    lookup
}

pub(crate) fn org_row(fields: &str) -> OrganizationTable {
    serde_json::from_str(fields).expect("the organization fixture decodes")
}

type CacheRead<T> = Result<Option<T>, AuthCacheError>;
type CacheWrite = Result<(), AuthCacheError>;

#[async_trait]
impl OrgCache for OrgWorld {
    async fn read_org(&self, key: &str, _local_only: bool) -> CacheRead<OrganizationTable> {
        let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(cached.get(key).cloned())
    }

    async fn write_org(&self, key: &str, org: &OrganizationTable, _ttl: f64) -> CacheWrite {
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        cached.insert(key.to_owned(), org.clone());
        drop(cached);
        Ok(())
    }

    async fn read_project(&self, key: &str) -> CacheRead<CachedProject> {
        let projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(projects
            .get(key)
            .and_then(|text| serde_json::from_str(text).ok()))
    }

    async fn write_project(&self, key: &str, project: &CachedProject, _ttl: f64) -> CacheWrite {
        let text = serde_json::to_string(project).unwrap_or_default();
        let mut projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        projects.insert(key.to_owned(), text);
        drop(projects);
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl OrgStore for OrgWorld {
    async fn find_org(&self, org_id: &str, include_budget: bool) -> StoreAnswer<Option<String>> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        let found = self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|row| row.organization_id.as_deref() == Some(org_id))
            .cloned();
        Ok(found.map(|mut row| {
            if !include_budget {
                row.litellm_budget_table = None;
            }
            serde_json::to_string(&row).expect("the organization fixture serializes")
        }))
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}
