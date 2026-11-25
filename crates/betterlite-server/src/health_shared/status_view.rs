use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use super::manager::SharedHealthCheckManager;

// get_health_check_status' dict: four fixed keys, then the lock and cache facts only a configured Redis answers.
#[derive(Clone, Debug)]
pub struct SharedHealthStatus {
    pub pod_id: String,
    pub redis_available: bool,
    pub lock_ttl: i64,
    pub cache_ttl: i64,
    pub lock_owner: Option<Json>,
    pub lock_in_progress: Option<bool>,
    pub cache_available: Option<bool>,
    pub cache_age_seconds: Option<f64>,
    pub last_checked_by: Option<String>,
    pub error: Option<String>,
}

impl SharedHealthStatus {
    #[must_use]
    pub fn to_json(&self) -> JsonObject {
        let mut status = JsonObject::new();
        status.insert("pod_id".to_owned(), Json::Text(self.pod_id.clone()));
        status.insert(
            "redis_available".to_owned(),
            Json::Bool(self.redis_available),
        );
        status.insert(
            "lock_ttl".to_owned(),
            Json::Number(Number::from(self.lock_ttl)),
        );
        status.insert(
            "cache_ttl".to_owned(),
            Json::Number(Number::from(self.cache_ttl)),
        );
        if let Some(error) = &self.error {
            status.insert("error".to_owned(), Json::Text(error.clone()));
            return status;
        }
        if !self.redis_available {
            return status;
        }
        let owner = self.lock_owner.clone().unwrap_or(Json::Null);
        status.insert("lock_owner".to_owned(), owner);
        let in_progress = self.lock_in_progress.unwrap_or(false);
        status.insert("lock_in_progress".to_owned(), Json::Bool(in_progress));
        let available = self.cache_available.unwrap_or(false);
        status.insert("cache_available".to_owned(), Json::Bool(available));
        if let Some(age) = self.cache_age_seconds {
            let age_json = Number::from_f64(age).map_or_else(|| Json::Null, Json::Number);
            status.insert("cache_age_seconds".to_owned(), age_json);
            let checked_by = self.last_checked_by.clone().map_or(Json::Null, Json::Text);
            status.insert("last_checked_by".to_owned(), checked_by);
        }
        status
    }
}

// The route's status provider: whoever can answer get_health_check_status behind the auth stage.
#[async_trait]
pub trait SharedHealthStatusSource: Send + Sync {
    async fn status(&self) -> Result<JsonObject, SharedHealthStatusError>;
}

// A status call that raised before it could answer; the route renders it as LiteLLM's 500.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct SharedHealthStatusError {
    pub message: String,
}

#[async_trait]
impl SharedHealthStatusSource for SharedHealthCheckManager {
    async fn status(&self) -> Result<JsonObject, SharedHealthStatusError> {
        Ok(self.get_health_check_status().await.to_json())
    }
}
