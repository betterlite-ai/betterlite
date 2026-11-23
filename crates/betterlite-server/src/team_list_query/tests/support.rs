use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{
    BudgetResetCalculator, DefaultTeamEnrollment, NoDefaultTeams, PortError,
};
use betterlite_auth::auth_user_lookup::store::{DatabaseUserStore, UserStore};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;
use tokio_util::task::TaskTracker;

use crate::key_query_plan::sql::SqlValue;
use crate::team_list_query::conditions::{TeamIdSearchMatch, TeamListFilters, TeamListWhereClause};
use crate::team_list_query::sql::render_where;
use crate::team_list_query::{TeamListQueryError, TeamListRejection};

pub(super) fn data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(super) fn record(text: &str) -> TeamRecord {
    serde_json::from_str(text).unwrap()
}

pub(super) fn user(text: &str) -> LiteLLMUserTable {
    serde_json::from_str(text).unwrap()
}

// the rejected error a guard answers, so a test can read the status and the message
pub(super) fn rejected<Answer: std::fmt::Debug>(
    result: Result<Answer, TeamListQueryError>,
) -> TeamListRejection {
    match result {
        Err(TeamListQueryError::Rejected(rejection)) => rejection,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

pub(super) fn auth(text: &str) -> UserAPIKeyAuth {
    let JsonData::Object(fields) = data(text) else {
        panic!("a caller fixture is an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn team_filters(
    team_id: Option<&str>,
    team_alias: Option<&str>,
    organization_id: Option<&str>,
    user_id: Option<&str>,
) -> TeamListFilters {
    TeamListFilters {
        team_id: team_id.map(str::to_owned),
        team_alias: team_alias.map(str::to_owned),
        organization_id: organization_id.map(str::to_owned),
        user_id: user_id.map(str::to_owned),
        search: None,
        search_team_id_match: TeamIdSearchMatch::Exact,
        use_deleted_table: false,
    }
}

pub(super) fn searching(search: &str, prefix: bool) -> TeamListFilters {
    let search_team_id_match = if prefix {
        TeamIdSearchMatch::Prefix
    } else {
        TeamIdSearchMatch::Exact
    };
    TeamListFilters {
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_id: None,
        search: Some(search.to_owned()),
        search_team_id_match,
        use_deleted_table: false,
    }
}

// the cache tier get_user_object reads first; reads and writes are counted to prove whether a lookup ran
#[derive(Default)]
pub(super) struct RecordingCache {
    pub(super) user: Option<LiteLLMUserTable>,
    pub(super) reads: Mutex<Vec<String>>,
    pub(super) writes: AtomicUsize,
}

impl RecordingCache {
    pub(super) fn read_keys(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl UserCache for RecordingCache {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        self.reads.lock().unwrap().push(key.to_owned());
        Ok(self.user.clone())
    }

    async fn write(
        &self,
        _key: &str,
        _user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

struct UnusedReset;

impl BudgetResetCalculator for UnusedReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new(
            "a lookup without upsert never computes a reset time",
        ))
    }
}

pub(super) fn lookup(cache: &Arc<RecordingCache>, store: Option<Arc<dyn UserStore>>) -> UserLookup {
    let cache_port: Arc<dyn UserCache> = Arc::<RecordingCache>::clone(cache);
    let enrollment: Arc<dyn DefaultTeamEnrollment> = Arc::new(NoDefaultTeams);
    UserLookup {
        cache: cache_port,
        store,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(UnusedReset),
        enrollment,
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    }
}

pub(super) fn user_store(reader: &PostgresStore, writer: &PostgresStore) -> Arc<DatabaseUserStore> {
    Arc::new(DatabaseUserStore {
        reader: reader.clone(),
        writer: writer.clone(),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    })
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Fixture {
    postgres: PostgresFixture,
    pub(super) store: PostgresStore,
    availability: Availability,
}

impl Fixture {
    pub(super) const fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    pub(super) async fn matching_team_ids(
        &self,
        clause: &TeamListWhereClause,
        use_deleted_table: bool,
    ) -> Vec<String> {
        let rendered = render_where(clause);
        let table = if use_deleted_table {
            "LiteLLM_DeletedTeamTable"
        } else {
            "LiteLLM_TeamTable"
        };
        let sql = format!(
            "SELECT DISTINCT t.\"team_id\" FROM \"{table}\" t WHERE {} ORDER BY t.\"team_id\"",
            rendered.sql
        );
        let params: Vec<&(dyn ToSql + Sync)> = rendered.params.iter().map(SqlValue::bind).collect();
        let session = self.store.session().await.unwrap();
        let rows = session.query(&sql, &params).await.unwrap();
        rows.iter().map(|row| row.get(0)).collect()
    }

    pub(super) async fn close(self) {
        if let Some(pool) = self.store.pool.as_ref() {
            pool.close();
        }
        self.postgres.close_async().await.unwrap();
    }
}

pub(super) async fn database(seed: &str) -> Fixture {
    let url = Environment::parse_from(["team-list-query-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
    let postgres = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(postgres.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    if !seed.is_empty() {
        pool.get().await.unwrap().batch_execute(seed).await.unwrap();
    }
    Fixture {
        postgres,
        store: PostgresStore { pool: Some(pool) },
        availability: Availability::default(),
    }
}

fn text_array(values: &[&str]) -> String {
    if values.is_empty() {
        return String::from("ARRAY[]::text[]");
    }
    let items: Vec<String> = values.iter().map(|value| format!("'{value}'")).collect();
    format!("ARRAY[{}]::text[]", items.join(","))
}

pub(super) fn budget_insert(budget_id: &str) -> String {
    format!(
        "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id, created_by, updated_by) VALUES ('{budget_id}', 'test', 'test');"
    )
}

pub(super) fn organization_insert(organization_id: &str) -> String {
    let budget_id = format!("budget-{organization_id}");
    format!(
        "{}INSERT INTO \"LiteLLM_OrganizationTable\" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('{organization_id}', '{organization_id}', '{budget_id}', 'test', 'test');",
        budget_insert(&budget_id)
    )
}

pub(super) fn membership_insert(user_id: &str, organization_id: &str, role: &str) -> String {
    format!(
        "INSERT INTO \"LiteLLM_OrganizationMembership\" (user_id, organization_id, user_role) VALUES ('{user_id}', '{organization_id}', '{role}');"
    )
}

pub(super) fn user_insert(user_id: &str, teams: &[&str]) -> String {
    format!(
        "INSERT INTO \"LiteLLM_UserTable\" (user_id, teams) VALUES ('{user_id}', {});",
        text_array(teams)
    )
}

pub(super) fn team_insert(
    team_id: &str,
    team_alias: &str,
    organization_id: Option<&str>,
    members: &[&str],
    roster: &[&str],
) -> String {
    let organization =
        organization_id.map_or_else(|| String::from("NULL"), |org_id| format!("'{org_id}'"));
    let roster_items: Vec<String> = roster
        .iter()
        .map(|user_id| format!("{{\"user_id\":\"{user_id}\",\"role\":\"user\"}}"))
        .collect();
    format!(
        "INSERT INTO \"LiteLLM_TeamTable\" (team_id, team_alias, organization_id, admins, members, members_with_roles, models) VALUES ('{team_id}', '{team_alias}', {organization}, ARRAY[]::text[], {}, '[{}]'::jsonb, ARRAY[]::text[]);",
        text_array(members),
        roster_items.join(",")
    )
}

pub(super) fn key_insert(token: &str, team_id: &str) -> String {
    format!(
        "INSERT INTO \"LiteLLM_VerificationToken\" (token, team_id) VALUES ('{token}', '{team_id}');"
    )
}

pub(super) fn access_group_insert(group_id: &str, name: &str, models: &[&str]) -> String {
    format!(
        "INSERT INTO \"LiteLLM_AccessGroupTable\" (access_group_id, access_group_name, access_model_names) VALUES ('{group_id}', '{name}', {});",
        text_array(models)
    )
}

pub(super) fn deleted_team_insert(team_id: &str, members: &[&str]) -> String {
    format!(
        "INSERT INTO \"LiteLLM_DeletedTeamTable\" (id, team_id, members) VALUES ('deleted-{team_id}', '{team_id}', {});",
        text_array(members)
    )
}
