use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::combined_key_read::error::{DatabaseFailure, KeyReadError};
use betterlite_db::combined_key_read::view::CombinedKeyView;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use opentelemetry::trace::SpanContext;
use rust_decimal::Decimal;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::task::TaskTracker;

use crate::auth_identity::errors::AuthError as IdentityError;
use crate::auth_invalidation::error::InvalidationError;
use crate::auth_key_lookup::KeyLookups;
use crate::auth_key_lookup::cache::{KeyCache, KeyCacheWriter};
use crate::auth_key_lookup::jwt::{JwtError, JwtMappingSource};
use crate::auth_key_lookup::key_object::KeyRequest;
use crate::auth_key_lookup::permission::KeyPermissionLookup;
use crate::auth_key_lookup::reader::{KeyReader, KeySource};
use crate::auth_key_lookup::session::refresh_session_token_grants;
use crate::auth_prefetch::permission::lookup::PermissionError;
use crate::auth_read::jwt_rows::{JwtMappingFilter, JwtMappingRow};
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::TeamRequest;
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::UserLookupRequest;
use crate::identity_grants::denial::PublicDenial;
use crate::identity_grants::loaders::{MembershipLoader, TeamLoader, UserLoader};
use crate::identity_grants::resolver::GrantResolver;
use crate::membership_load::error::MembershipLoadError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(super) const KEY_ROW: &str = r#"{"token":"hashed-key","key_name":"key-1","user_id":"u1","team_id":"t1","models":["gpt-4"],"spend":1.5}"#;

pub(super) fn auth_from_fields(text: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(text).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn view_from_text(text: &str) -> CombinedKeyView {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(text).unwrap();
    CombinedKeyView::construct(fields, Decimal::ZERO).unwrap()
}

pub(super) fn auth_text(auth: &UserAPIKeyAuth) -> String {
    serde_json::to_string(auth).unwrap()
}

pub(super) fn fault(error_type: ErrorType, message: &str) -> KeyReadError {
    KeyReadError::Database(DatabaseFailure::Reported(DbError {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }))
}

pub(super) fn transport() -> KeyReadError {
    fault(ErrorType::Connect, "connection closed")
}

pub(super) struct FakeSource {
    answers: Mutex<VecDeque<Result<Option<CombinedKeyView>, KeyReadError>>>,
    calls: Mutex<Vec<(String, bool)>>,
}

impl FakeSource {
    pub(super) fn new(answers: Vec<Result<Option<CombinedKeyView>, KeyReadError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<(String, bool)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl KeySource for FakeSource {
    async fn read_key(
        &self,
        hashed_token: &str,
        use_writer: bool,
        _parent_otel_span: Option<SpanContext>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((hashed_token.to_owned(), use_writer));
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(None))
    }
}

pub(super) type ReconnectCalls = Arc<Mutex<Vec<(String, f64, f64)>>>;

pub(super) struct FakeReconnect {
    pub(super) timeout: Option<f64>,
    pub(super) lock: Option<f64>,
    pub(super) answer: Result<bool, DbError>,
    pub(super) calls: ReconnectCalls,
    pub(super) delay: Option<Duration>,
}

pub(super) fn fake_reconnect(answer: Result<bool, DbError>) -> (FakeReconnect, ReconnectCalls) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        FakeReconnect {
            timeout: None,
            lock: None,
            answer,
            calls: Arc::clone(&calls),
            delay: None,
        },
        calls,
    )
}

pub(super) fn reconnect_calls(calls: &ReconnectCalls) -> Vec<(String, f64, f64)> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

impl DbReconnect for FakeReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        self.timeout
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        self.lock
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((reason.to_owned(), timeout_seconds, lock_timeout_seconds));
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            self.answer.clone()
        })
    }
}

pub(super) fn reader(source: Arc<FakeSource>, reconnect: Option<FakeReconnect>) -> Arc<KeyReader> {
    Arc::new(KeyReader {
        source,
        reconnect: reconnect.map(|client| {
            Arc::new(AsyncMutex::new(
                Box::new(client) as Box<dyn DbReconnect + Send>
            ))
        }),
        gate: Arc::new(LoopBoundSemaphore {
            value: 8,
            slot: Mutex::new(None),
        }),
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
        default_deadline_seconds: 5.0,
    })
}

pub(super) struct FakeCache {
    hit: Option<UserAPIKeyAuth>,
    reads: Mutex<Vec<String>>,
}

impl FakeCache {
    pub(super) fn new(hit: Option<UserAPIKeyAuth>) -> Arc<Self> {
        Arc::new(Self {
            hit,
            reads: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl KeyCache for FakeCache {
    async fn read_key(&self, hashed_token: &str) -> Option<UserAPIKeyAuth> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hashed_token.to_owned());
        self.hit.clone()
    }
}

pub(super) struct FakeWriter {
    writes: Mutex<Vec<String>>,
    seen_permission: Mutex<Vec<bool>>,
    fail: bool,
}

impl FakeWriter {
    pub(super) fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self {
            writes: Mutex::new(Vec::new()),
            seen_permission: Mutex::new(Vec::new()),
            fail,
        })
    }

    pub(super) fn writes(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn seen_permission(&self) -> Vec<bool> {
        self.seen_permission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl KeyCacheWriter for FakeWriter {
    async fn write_key(
        &self,
        hashed_token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hashed_token.to_owned());
        self.seen_permission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(auth.view.token.object_permission.is_some());
        auth.view.last_refreshed_at = Some(Decimal::ZERO);
        if self.fail {
            return Err(InvalidationError::Timestamp(Decimal::ZERO));
        }
        Ok(())
    }
}

pub(super) fn permission_table() -> LiteLLMObjectPermissionTable {
    serde_json::from_str(r#"{"object_permission_id":"op-1","models":["gpt-4"]}"#).unwrap()
}

pub(super) struct FakePermissions {
    answers: Mutex<VecDeque<Result<Option<LiteLLMObjectPermissionTable>, PermissionError>>>,
    calls: Mutex<Vec<(String, bool)>>,
}

impl FakePermissions {
    pub(super) fn new(
        answers: Vec<Result<Option<LiteLLMObjectPermissionTable>, PermissionError>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<(String, bool)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl KeyPermissionLookup for FakePermissions {
    async fn permission(
        &self,
        object_permission_id: &str,
        check_db_only: bool,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((object_permission_id.to_owned(), check_db_only));
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(None))
    }
}

pub(super) fn lookups(
    reader: Option<Arc<KeyReader>>,
    cache: Arc<FakeCache>,
    writer: Arc<FakeWriter>,
    permissions: Option<Arc<FakePermissions>>,
) -> KeyLookups {
    KeyLookups {
        reader,
        cache,
        writer,
        permissions: permissions.map(|source| source as Arc<dyn KeyPermissionLookup>),
    }
}

pub(super) fn key_request(hashed_token: &str) -> KeyRequest<'_> {
    KeyRequest {
        hashed_token,
        parent_otel_span: None,
        check_cache_only: None,
        check_db_only: false,
    }
}

pub(super) struct FakeMappings {
    filter: Mutex<Option<JwtMappingFilter>>,
    rows: Vec<JwtMappingRow>,
    token_calls: Mutex<Vec<Vec<String>>>,
    found: Option<JwtMappingRow>,
}

impl FakeMappings {
    pub(super) fn new(rows: Vec<JwtMappingRow>, found: Option<JwtMappingRow>) -> Arc<Self> {
        Arc::new(Self {
            filter: Mutex::new(None),
            rows,
            token_calls: Mutex::new(Vec::new()),
            found,
        })
    }

    pub(super) fn filter(&self) -> Option<JwtMappingFilter> {
        self.filter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn token_calls(&self) -> Vec<Vec<String>> {
        self.token_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl JwtMappingSource for FakeMappings {
    async fn find_active_mapping(
        &self,
        filter: &JwtMappingFilter,
    ) -> Result<Option<JwtMappingRow>, JwtError> {
        *self.filter.lock().unwrap_or_else(PoisonError::into_inner) = Some(filter.clone());
        Ok(self.found.clone())
    }

    async fn mappings_for_tokens(
        &self,
        hashed_tokens: &[String],
    ) -> Result<Vec<JwtMappingRow>, JwtError> {
        self.token_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hashed_tokens.to_vec());
        Ok(self.rows.clone())
    }
}

pub(super) fn mapping(
    token: &str,
    claim_name: &str,
    claim_value: &str,
    issuer: &str,
) -> JwtMappingRow {
    JwtMappingRow {
        token: token.to_owned(),
        jwt_issuer: issuer.to_owned(),
        jwt_claim_name: claim_name.to_owned(),
        jwt_claim_value: claim_value.to_owned(),
    }
}

pub(super) enum UserAnswer {
    Found(Box<LiteLLMUserTable>),
    Missing,
    Gone,
    Unavailable,
}

pub(super) struct FakeUsers(pub(super) UserAnswer);

#[async_trait]
impl UserLoader for FakeUsers {
    async fn load_user(
        &self,
        _request: UserLookupRequest<'_>,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        match &self.0 {
            UserAnswer::Found(user) => Ok(Some((**user).clone())),
            UserAnswer::Missing => Ok(None),
            UserAnswer::Gone => Err(UserLookupError::NotFound(IdentityError::UserNotFound {
                user_id: "u1".to_owned(),
            })),
            UserAnswer::Unavailable => Err(UserLookupError::NoDatabase),
        }
    }
}

pub(super) struct FakeTeams(pub(super) Option<LiteLLMTeamTableCachedObj>);

#[async_trait]
impl TeamLoader for FakeTeams {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        let team_id = request.team_id.to_owned();
        match &self.0 {
            Some(team) => Ok(team.clone()),
            None => Err(TeamLookupError::TeamNotFound { team_id }),
        }
    }
}

pub(super) struct FakeMembers(pub(super) Option<LiteLLMTeamMembership>);

#[async_trait]
impl MembershipLoader for FakeMembers {
    async fn load_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Ok(self.0.clone())
    }
}

pub(super) fn user_row(text: &str) -> LiteLLMUserTable {
    serde_json::from_str(text).unwrap()
}

pub(super) fn team_row(text: &str) -> LiteLLMTeamTableCachedObj {
    serde_json::from_str(text).unwrap()
}

pub(super) async fn refresh(
    auth: &UserAPIKeyAuth,
    users: &FakeUsers,
    teams: &FakeTeams,
    members: &FakeMembers,
) -> Result<UserAPIKeyAuth, PublicDenial> {
    let resolver = GrantResolver {
        users,
        teams,
        memberships: members,
    };
    refresh_session_token_grants(&resolver, auth).await
}

pub(super) fn team_session_auth() -> UserAPIKeyAuth {
    auth_from_fields(
        r#"{"token":"hashed-key","key_name":"cli-session","user_id":"u1","team_id":"t1","models":["minted"],"user_role":"internal_user","is_session_token":true}"#,
    )
}

pub(super) fn plain_session_auth() -> UserAPIKeyAuth {
    auth_from_fields(
        r#"{"token":"hashed-key","key_name":"cli-session","user_id":"u1","models":["minted"],"user_role":"internal_user","is_session_token":true}"#,
    )
}
