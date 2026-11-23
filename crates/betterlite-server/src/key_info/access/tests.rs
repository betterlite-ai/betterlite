#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{
    StoreAnswer, TeamCache, TeamCacheError, TeamStore, TeamStoreError,
};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::decoder::TeamRowError;
use rstest::rstest;
use sha2::{Digest as _, Sha256};

use super::can_user_query_key_info;
use crate::team_access::error::TeamAccessError;

const KEY_TEAM: &str =
    r#"{"team_id":"team-1","members_with_roles":[{"user_id":"member-1","role":"user"}]}"#;
const STALE_TEAM: &str =
    r#"{"team_id":"team-1","members_with_roles":[{"user_id":"stale-1","role":"admin"}]}"#;

// the cache tier and the team table behind get_team_object, counting every read
struct World {
    cached: Mutex<Option<LiteLLMTeamTableCachedObj>>,
    row: Mutex<Option<JsonData>>,
    fail_store: AtomicBool,
    cache_reads: AtomicUsize,
    store_reads: Mutex<Vec<bool>>,
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn fakes(cached: Option<&str>, row: Option<&str>) -> Arc<World> {
    let cached_team = cached.map(|text| LiteLLMTeamTableCachedObj::try_from(json(text)).unwrap());
    Arc::new(World {
        cached: Mutex::new(cached_team),
        row: Mutex::new(row.map(json)),
        fail_store: AtomicBool::new(false),
        cache_reads: AtomicUsize::new(0),
        store_reads: Mutex::new(Vec::new()),
    })
}

fn team_lookup(world: &Arc<World>) -> TeamLookup {
    TeamLookup {
        cache: Arc::<World>::clone(world),
        permission_cache: Arc::<World>::clone(world),
        database: Some(TeamDatabase {
            store: Arc::<World>::clone(world),
            permission_reader: Arc::<World>::clone(world),
            permission_writer: Arc::<World>::clone(world),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

fn caller(role: Option<UserRole>, api_key: Option<&str>, user_id: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(role) = role {
        fields.insert("user_role".to_owned(), JsonData::String(role.to_string()));
    }
    if let Some(api_key) = api_key {
        fields.insert("api_key".to_owned(), JsonData::String(api_key.to_owned()));
    }
    if let Some(user_id) = user_id {
        fields.insert("user_id".to_owned(), JsonData::String(user_id.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn key_row(user_id: Option<&str>, team_id: Option<&str>) -> LiteLLMVerificationToken {
    let text = format!(
        r#"{{"user_id":{},"team_id":{}}}"#,
        serde_json::to_string(&user_id).unwrap(),
        serde_json::to_string(&team_id).unwrap()
    );
    serde_json::from_str(&text).unwrap()
}

fn sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[rstest]
#[case::admin(UserRole::ProxyAdmin)]
#[case::viewer(UserRole::ProxyAdminViewer)]
#[tokio::test]
async fn admin_and_viewer_pass(#[case] role: UserRole) {
    let world = fakes(Some(STALE_TEAM), Some(KEY_TEAM));
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), Some("team-1"));
    let auth = caller(Some(role), Some("sk-outsider"), Some("outsider"));
    assert!(
        can_user_query_key_info(&auth, None, &row, &lookup)
            .await
            .unwrap()
    );
    assert_eq!(world.cache_reads.load(Ordering::SeqCst), 0);
    assert!(world.store_reads.lock().unwrap().is_empty());
}

#[rstest]
#[case::hashed(true)]
#[case::raw(false)]
#[tokio::test]
async fn same_api_key(#[case] hashed: bool) {
    let world = fakes(None, None);
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), None);
    let auth = caller(
        Some(UserRole::InternalUser),
        Some("sk-sample-key"),
        Some("outsider"),
    );
    let key = if hashed {
        sha256("sk-sample-key")
    } else {
        "sk-sample-key".to_owned()
    };
    assert_eq!(
        can_user_query_key_info(&auth, Some(&key), &row, &lookup)
            .await
            .unwrap(),
        hashed
    );
}

#[tokio::test]
async fn both_keys_absent() {
    let world = fakes(None, None);
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), None);
    let auth = caller(None, None, Some("outsider"));
    assert!(
        can_user_query_key_info(&auth, None, &row, &lookup)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn owner_match() {
    let world = fakes(None, None);
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), None);
    let auth = caller(
        Some(UserRole::InternalUser),
        Some("sk-other"),
        Some("owner-1"),
    );
    assert!(
        can_user_query_key_info(&auth, None, &row, &lookup)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn ownerless_key_and_caller() {
    let world = fakes(None, None);
    let lookup = team_lookup(&world);
    let row = key_row(None, None);
    let auth = caller(Some(UserRole::InternalUser), Some("sk-other"), None);
    assert!(
        can_user_query_key_info(&auth, None, &row, &lookup)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn member_of_key_team() {
    let world = fakes(Some(STALE_TEAM), Some(KEY_TEAM));
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), Some("team-1"));
    let auth = caller(
        Some(UserRole::InternalUser),
        Some("sk-member"),
        Some("member-1"),
    );
    assert!(
        can_user_query_key_info(&auth, None, &row, &lookup)
            .await
            .unwrap()
    );
    assert_eq!(*world.store_reads.lock().unwrap(), [true]);
    assert_eq!(world.cache_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn not_in_key_team() {
    let world = fakes(Some(KEY_TEAM), Some(STALE_TEAM));
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), Some("team-1"));
    let auth = caller(
        Some(UserRole::InternalUser),
        Some("sk-outsider"),
        Some("member-1"),
    );
    assert!(
        !can_user_query_key_info(&auth, None, &row, &lookup)
            .await
            .unwrap()
    );
    assert_eq!(*world.store_reads.lock().unwrap(), [true]);
}

#[tokio::test]
async fn personal_key_no_lookup() {
    let world = fakes(Some(KEY_TEAM), Some(KEY_TEAM));
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), None);
    let auth = caller(
        Some(UserRole::InternalUser),
        Some("sk-outsider"),
        Some("member-1"),
    );
    assert!(
        !can_user_query_key_info(&auth, None, &row, &lookup)
            .await
            .unwrap()
    );
    assert!(world.store_reads.lock().unwrap().is_empty());
    assert_eq!(world.cache_reads.load(Ordering::SeqCst), 0);
}

#[derive(Clone, Copy, Debug)]
enum Granted {
    Admin,
    ApiKey,
    Owner,
}

#[rstest]
#[case::admin(Granted::Admin)]
#[case::api_key(Granted::ApiKey)]
#[case::owner(Granted::Owner)]
#[tokio::test]
async fn short_circuit_skips_team_read(#[case] granted: Granted) {
    let world = fakes(Some(KEY_TEAM), Some(KEY_TEAM));
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), Some("team-1"));
    let (auth, key) = match granted {
        Granted::Admin => (
            caller(
                Some(UserRole::ProxyAdmin),
                Some("sk-outsider"),
                Some("outsider"),
            ),
            None,
        ),
        Granted::ApiKey => (
            caller(
                Some(UserRole::InternalUser),
                Some("sk-sample-key"),
                Some("outsider"),
            ),
            Some(sha256("sk-sample-key")),
        ),
        Granted::Owner => (
            caller(
                Some(UserRole::InternalUser),
                Some("sk-outsider"),
                Some("owner-1"),
            ),
            None,
        ),
    };
    assert!(
        can_user_query_key_info(&auth, key.as_deref(), &row, &lookup)
            .await
            .unwrap()
    );
    assert!(world.store_reads.lock().unwrap().is_empty());
    assert_eq!(world.cache_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn team_read_error_propagates() {
    let world = fakes(None, Some(KEY_TEAM));
    world.fail_store.store(true, Ordering::SeqCst);
    let lookup = team_lookup(&world);
    let row = key_row(Some("owner-1"), Some("team-1"));
    let auth = caller(
        Some(UserRole::InternalUser),
        Some("sk-outsider"),
        Some("member-1"),
    );
    let error = can_user_query_key_info(&auth, None, &row, &lookup)
        .await
        .unwrap_err();
    assert!(matches!(error, TeamAccessError::TeamLookup(_)));
}

#[async_trait]
impl TeamCache for World {
    async fn read_team(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        self.cache_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.cached.lock().unwrap().clone())
    }

    async fn write_team(
        &self,
        _key: &str,
        _team: &LiteLLMTeamTableCachedObj,
        _ttl: f64,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn cache_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn read_access_group(
        &self,
        _key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(None)
    }

    async fn cache_access_group(
        &self,
        _access_group_id: &str,
        _table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[async_trait]
impl TeamStore for World {
    async fn find_team(&self, _team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        self.store_reads.lock().unwrap().push(use_writer);
        if self.fail_store.load(Ordering::SeqCst) {
            return Err(TeamStoreError::Row(Box::new(TeamRowError::NotAnObject)));
        }
        Ok(self.row.lock().unwrap().clone())
    }

    async fn find_by_alias(&self, _team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        Ok(Vec::new())
    }

    async fn find_access_group(
        &self,
        _access_group_id: &str,
        _use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionCache for World {
    async fn read(&self, _key: &str) -> PermissionAnswer {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        _ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl ObjectPermissionStore for World {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}
