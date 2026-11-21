mod tests;

// A failed health read; its text is what the debug log prints, and every failure enforces limits
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct DeploymentHealthError {
    pub message: String,
}

// One router deployment of the model; the id is the router's model_info.id and may be missing
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentRef {
    pub id: Option<String>,
}

// LiteLLM reaches proxy_server.llm_router and the failure counter directly; this seam keeps both out of the crate
#[async_trait::async_trait]
pub trait DeploymentHealthSource: Send + Sync {
    async fn deployments_for_model(
        &self,
        model: &str,
    ) -> Result<Option<Vec<DeploymentRef>>, DeploymentHealthError>;

    async fn failures_this_minute(&self, deployment_id: &str)
    -> Result<i64, DeploymentHealthError>;
}

#[derive(Clone, Copy)]
pub struct DeploymentHealthPolicy<'source> {
    pub source: &'source dyn DeploymentHealthSource,
    pub error_threshold_per_minute: i64,
}

impl DeploymentHealthPolicy<'_> {
    // LiteLLM's _check_model_has_recent_failures: strict greater-than, and any read failure enforces limits
    pub async fn model_has_recent_failures(&self, model: &str) -> bool {
        let deployments = match self.source.deployments_for_model(model).await {
            Ok(Some(deployments)) => deployments,
            Ok(None) => return false,
            Err(error) => return enforce_on_error(&error),
        };
        for deployment in deployments {
            match self
                .deployment_has_failures(model, deployment.id.as_deref())
                .await
            {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => return enforce_on_error(&error),
            }
        }
        tracing::debug!(
            "[Dynamic Rate Limit] No failures detected for model {model} - allowing dynamic exceeding"
        );
        false
    }

    async fn deployment_has_failures(
        &self,
        model: &str,
        deployment_id: Option<&str>,
    ) -> Result<bool, DeploymentHealthError> {
        let Some(id) = deployment_id.filter(|value| !value.is_empty()) else {
            return Ok(false);
        };
        let failure_count = self.source.failures_this_minute(id).await?;
        if failure_count <= self.error_threshold_per_minute {
            return Ok(false);
        }
        tracing::debug!(
            "[Dynamic Rate Limit] Deployment {id} has {failure_count} failures in current minute - enforcing rate limits for model {model}"
        );
        Ok(true)
    }
}

// LiteLLM's except branch: a failed check must enforce limits, never relax them
fn enforce_on_error(error: &DeploymentHealthError) -> bool {
    tracing::debug!("Error checking model failure status: {error}, defaulting to enforce limits");
    true
}
