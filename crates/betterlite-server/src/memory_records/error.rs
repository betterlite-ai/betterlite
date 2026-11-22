use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use std::error::Error;

// the HTTPException LiteLLM raises: a status and the detail FastAPI answers
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct MemoryHttpError {
    pub status: StatusCode,
    pub detail: String,
}

// create_memory and upsert_memory answer this for a key that already exists, whoever owns the row
#[must_use]
pub fn duplicate_key_error(key: &str) -> MemoryHttpError {
    MemoryHttpError {
        status: StatusCode::CONFLICT,
        detail: format!("Memory with key '{key}' already exists."),
    }
}

// memory_endpoints.py _find_memory_for_caller: no visible row carries the key
#[must_use]
pub fn not_found_error(key: &str) -> MemoryHttpError {
    MemoryHttpError {
        status: StatusCode::NOT_FOUND,
        detail: format!("Memory with key '{key}' not found"),
    }
}

// memory_endpoints.py _require_prisma: no database is a 500 that names the missing configuration
pub fn require_database<T>(database: Option<T>) -> Result<T, MemoryHttpError> {
    database.ok_or_else(|| MemoryHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        detail: CommonProxyErrors::DbNotConnectedError.as_str().to_owned(),
    })
}

// memory_endpoints.py _internal_error: log the real exception, answer a detail that names no table
#[must_use]
pub fn internal_error(
    log_message: &str,
    source: &(dyn Error + 'static),
    default_detail: &str,
) -> MemoryHttpError {
    tracing::error!("{log_message}: {source}");
    MemoryHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        detail: default_detail.to_owned(),
    }
}
