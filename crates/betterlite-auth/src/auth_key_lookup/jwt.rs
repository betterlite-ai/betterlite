use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::postgres_store::errors::{DbError as NativeDbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_util::task::TaskTracker;

use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::jwt_rows::{JwtMappingFilter, JwtMappingRow, JwtRowError, JwtRows};
use crate::auth_read::repository::RepositoryTable;
use crate::auth_read::table::{AuthReadError, AuthTable as _, FindOne, Presence, ReadContext};

pub const JWT_LOOKUP: &str = "jwt_key_mapping";
const TOKEN_MAPPINGS_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_JWTKeyMapping\" AS entry WHERE entry.\"token\" = ANY($1::text[])";

#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("the JWT key mapping read failed")]
    Read(#[source] Box<AuthReadError<JwtRowError>>),
    #[error(transparent)]
    Lookup(#[from] DbLookupError),
}

// B726's JWTKeyMappingRepository as the two reads auth_checks performs.
#[async_trait]
pub trait JwtMappingSource: Send + Sync {
    async fn find_active_mapping(
        &self,
        filter: &JwtMappingFilter,
    ) -> Result<Option<JwtMappingRow>, JwtError>;
    async fn mappings_for_tokens(
        &self,
        hashed_tokens: &[String],
    ) -> Result<Vec<JwtMappingRow>, JwtError>;
}

// jwt_key_mapping_cache_key: an empty or absent issuer is the issuer-agnostic scope.
#[must_use]
pub fn jwt_key_mapping_cache_key(
    claim_name: &str,
    claim_value: &str,
    issuer: Option<&str>,
) -> String {
    issuer.filter(|value| !value.is_empty()).map_or_else(
        || format!("jwt_key_mapping:{claim_name}:{claim_value}"),
        |scope| format!("jwt_key_mapping:{scope}:{claim_name}:{claim_value}"),
    )
}

// ast-grep-ignore: no-long-names
pub async fn get_jwt_key_mapping_cache_keys_for_tokens(
    hashed_tokens: &[String],
    source: &dyn JwtMappingSource,
) -> Result<Vec<String>, JwtError> {
    match hashed_tokens {
        [] => Ok(Vec::new()),
        tokens => source
            .mappings_for_tokens(tokens)
            .await
            .map(|mappings| cache_keys(&mappings)),
    }
}

// get_jwt_key_mapping_object: one exact scope, no fallback to the global mapping.
pub async fn get_jwt_key_mapping_object(
    jwt_claim_name: &str,
    jwt_claim_value: &str,
    jwt_issuer: Option<&str>,
    source: &dyn JwtMappingSource,
) -> Result<Option<String>, JwtError> {
    Ok(source
        .find_active_mapping(&JwtMappingFilter {
            claim_name: jwt_claim_name.to_owned(),
            claim_value: jwt_claim_value.to_owned(),
            issuer: jwt_issuer.unwrap_or_default().to_owned(),
            active: true,
        })
        .await?
        .map(|mapping| mapping.token))
}

fn cache_keys(mappings: &[JwtMappingRow]) -> Vec<String> {
    mappings
        .iter()
        .map(|mapping| {
            jwt_key_mapping_cache_key(
                &mapping.jwt_claim_name,
                &mapping.jwt_claim_value,
                Some(&mapping.jwt_issuer),
            )
        })
        .collect()
}

// The live table over B714's routed reader; both reads carry B715's jwt_key_mapping deadline.
pub struct DatabaseJwtMappings {
    pub reader: PostgresStore,
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
}

impl DatabaseJwtMappings {
    fn context(&self) -> ReadContext<SystemClock> {
        ReadContext {
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        }
    }
}

#[async_trait]
impl JwtMappingSource for DatabaseJwtMappings {
    async fn find_active_mapping(
        &self,
        filter: &JwtMappingFilter,
    ) -> Result<Option<JwtMappingRow>, JwtError> {
        let session = self
            .reader
            .session()
            .await
            .map_err(|error| JwtError::Lookup(DbLookupError::Database(native_fault(&error))))?;
        let table = RepositoryTable {
            client: Some(Arc::new(session)),
            table: JwtRows,
        };
        let reader =
            AuthReadAdapter::JwtKeyMapping.open(Arc::new(table), String::new(), self.context());
        reader
            .find_first(FindOne {
                filter: filter.clone(),
                include: Presence::Omitted,
            })
            .await
            .map_err(|error| JwtError::Read(Box::new(error)))
    }

    async fn mappings_for_tokens(
        &self,
        hashed_tokens: &[String],
    ) -> Result<Vec<JwtMappingRow>, JwtError> {
        if hashed_tokens.is_empty() {
            return Ok(Vec::new());
        }
        let store = self.reader.clone();
        let tokens = hashed_tokens.to_vec();
        let lookup = async move { query_token_mappings(&store, &tokens).await };
        let request = LookupRequest {
            name: JWT_LOOKUP,
            deadline_seconds: None,
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: self.tracker.as_ref(),
        };
        bounded_db_lookup(lookup, request, &self.tasks)
            .await
            .map_err(JwtError::from)
    }
}

async fn query_token_mappings(
    store: &PostgresStore,
    tokens: &[String],
) -> Result<Vec<JwtMappingRow>, DbLookupError> {
    let client = store
        .session()
        .await
        .map_err(|error| DbLookupError::Database(native_fault(&error)))?;
    let rows = client
        .query(TOKEN_MAPPINGS_SQL, &[&tokens])
        .await
        .map_err(|source| DbLookupError::Database(native_fault(&backend(source))))?;
    let mut mappings = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row
            .try_get(0)
            .map_err(|source| DbLookupError::Database(native_fault(&backend(source))))?;
        mappings.push(decode_mapping(&text)?);
    }
    Ok(mappings)
}

fn decode_mapping(text: &str) -> Result<JwtMappingRow, DbLookupError> {
    serde_json::from_str(text).map_err(|error| {
        DbLookupError::Database(DbError {
            error_type: ErrorType::Data,
            message: format!("the JWT key mapping row did not decode: {error}"),
            code: None,
            meta_code: None,
            traceback_modules: Vec::new(),
            cause: None,
        })
    })
}

fn native_fault(error: &NativeDbError) -> DbError {
    let error_type = if is_native_transport_error(error) {
        ErrorType::Connect
    } else {
        ErrorType::Prisma
    };
    DbError {
        error_type,
        message: error.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
