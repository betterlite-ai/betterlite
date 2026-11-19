use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::settings::JsonValue;

use super::error::ConfigBootError;
use super::raw::is_configured;
use crate::job_scheduler::BoxFuture;

// B3250 owns init_policies; the port fixes LiteLLM's fail_on_error=True and this step picks validate_db
pub trait PolicyEngine: Send + Sync {
    fn init_policies<'config>(
        &'config self,
        policies: &'config JsonValue,
        policy_attachments: Option<&'config JsonValue>,
        validate_db: bool,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>>;
}

pub async fn init_policy_engine(
    config: Option<&ConfigMap>,
    has_db: bool,
    ports: &dyn PolicyEngine,
) -> Result<(), ConfigBootError> {
    let Some(config) = config else {
        tracing::debug!("Policy engine: config is None, skipping");
        return Ok(());
    };
    let Some(policies) = config.get("policies") else {
        tracing::debug!("Policy engine: no policies in config, skipping");
        return Ok(());
    };
    if !is_configured(policies) {
        tracing::debug!("Policy engine: no policies in config, skipping");
        return Ok(());
    }
    let count = match policies {
        JsonValue::Array(items) => items.len(),
        JsonValue::Object(fields) => fields.len(),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => 0,
    };
    tracing::info!("Policy engine: found {count} policies in config");
    let attachments = config.get("policy_attachments");
    ports.init_policies(policies, attachments, has_db).await
}

mod tests;
