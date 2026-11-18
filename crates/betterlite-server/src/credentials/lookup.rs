use betterlite_db::identity_rows::Payload;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentCredentials {
    pub model_name: String,
    pub credential_values: Payload,
}

pub trait DeploymentLookup: Send + Sync {
    // None when the deployment is missing or blocked, which LiteLLM answers with 404 Model not found
    fn deployment_credentials(&self, model_id: &str) -> Option<DeploymentCredentials>;
}
