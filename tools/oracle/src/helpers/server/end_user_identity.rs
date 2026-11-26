use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_cache::{AuthCacheError, WriteTtl};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::end_user_identity::cache::{VALIDATION_KEY_PREFIX, ValidationCache};
use betterlite_auth::end_user_identity::existence::{
    EndUserExistence, ExistenceError, UserExistence, UserProbe,
};
use betterlite_auth::end_user_identity::resolve::{
    DatabaseLookups, EndUserIdResolver, ResolveRequest,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Builder;

pub(crate) struct Op;

// The row an end-user lookup finds; a missing row and a swallowed read failure look the same to the resolver.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EndUserAnswer {
    Found,
    Missing,
}

// A user read failure raises in LiteLLM, so it stays distinct from a missing row.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum UserAnswer {
    Found,
    Missing,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    raw_end_user_id: Option<String>,
    key_end_user_budget_id: Option<String>,
    validate_in_db: bool,
    database: bool,
    proxy_default: Option<String>,
    cached: Option<Json>,
    end_user: EndUserAnswer,
    user: UserAnswer,
}

#[derive(Clone, Debug, Serialize)]
struct WriteRecord {
    key: String,
    value: Json,
    ttl: f64,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Arc::new(Fixture::new(input));
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(resolve(input, &fixture))
    }
}

async fn resolve(input: &Input, fixture: &Arc<Fixture>) -> HelperResult {
    let cache: Arc<dyn ValidationCache> = Arc::clone(fixture);
    let end_users: Arc<dyn EndUserExistence> = Arc::clone(fixture);
    let users: Arc<dyn UserExistence> = Arc::clone(fixture);
    let resolver = EndUserIdResolver {
        validate_end_user_id_in_db: input.validate_in_db,
        max_end_user_budget_id: input.proxy_default.clone(),
        lookups: input.database.then(|| DatabaseLookups { end_users, users }),
        cache,
    };
    let resolved = resolver
        .resolve(&ResolveRequest {
            raw_end_user_id: input.raw_end_user_id.as_deref(),
            key_end_user_budget_id: input.key_end_user_budget_id.as_deref(),
        })
        .await
        .or_value_error()?;
    Ok(json!({
        "value": resolved,
        "reads": fixture.reads(),
        "writes": fixture.writes(),
        "queries": fixture.queries(),
    }))
}

// The resolver's cache, end-user rows and user rows, journalled in call order.
#[derive(Debug)]
struct Fixture {
    end_user: EndUserAnswer,
    user: UserAnswer,
    values: Mutex<BTreeMap<String, Json>>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<WriteRecord>>,
    queries: Mutex<Vec<String>>,
}

impl Fixture {
    fn new(input: &Input) -> Self {
        let mut values = BTreeMap::new();
        if let (Some(id), Some(cached)) = (&input.raw_end_user_id, &input.cached) {
            let _previous = values.insert(format!("{VALIDATION_KEY_PREFIX}{id}"), cached.clone());
        }
        Self {
            end_user: input.end_user,
            user: input.user,
            values: Mutex::new(values),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            queries: Mutex::new(Vec::new()),
        }
    }

    fn record_query(&self, query: String) {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(query);
    }

    fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn writes(&self) -> Vec<WriteRecord> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn queries(&self) -> Vec<String> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl EndUserExistence for Fixture {
    async fn end_user_exists(&self, end_user_id: &str) -> Result<bool, ExistenceError> {
        self.record_query(format!("end_user:{end_user_id}"));
        Ok(matches!(self.end_user, EndUserAnswer::Found))
    }
}

#[async_trait]
impl UserExistence for Fixture {
    async fn user_exists(&self, probe: &UserProbe<'_>) -> Result<bool, ExistenceError> {
        self.record_query(format!("user:{}", probe.user_id));
        match self.user {
            UserAnswer::Found => Ok(true),
            UserAnswer::Missing => {
                if let Some(email) = probe.user_email {
                    self.record_query(format!("email:{email}"));
                }
                Ok(false)
            }
            UserAnswer::Error => Err(ExistenceError::User(UserLookupError::Unavailable(
                Box::new(IoError::other("the user row read failed")),
            ))),
        }
    }
}

#[async_trait]
impl ValidationCache for Fixture {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(cache_key.to_owned());
        let cached = self
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(cache_key)
            .cloned();
        Ok(cached)
    }

    async fn write(
        &self,
        cache_key: &str,
        value: &Json,
        ttl: WriteTtl,
    ) -> Result<(), AuthCacheError> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(WriteRecord {
                key: cache_key.to_owned(),
                value: value.clone(),
                ttl: ttl.seconds,
            });
        Ok(())
    }
}
