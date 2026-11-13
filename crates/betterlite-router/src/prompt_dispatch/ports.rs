use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_identity::spec::DeploymentSpec;

// The arguments of the logging object's get_chat_completion_prompt for one deployment.
pub struct CompilePromptCall<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub non_default_params: &'request JsonObject,
    pub prompt_id: Option<&'request Json>,
    pub prompt_variables: Option<&'request Json>,
    pub prompt_label: Option<&'request Json>,
    pub request: &'request JsonObject,
    pub injected_for_every_deployment: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledPrompt {
    pub model: String,
    pub messages: Vec<Json>,
    pub optional_params: JsonObject,
}

// The router services Router._prompt_management_factory calls; B016's router implements this trait.
#[async_trait]
pub trait PromptPorts: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type Response;

    // Router.async_get_available_deployment
    async fn select_deployment(
        &self,
        model: &str,
        messages: &[Json],
        specific_deployment: Option<&Json>,
        request: &JsonObject,
    ) -> Result<DeploymentSpec, Self::Error>;

    // Router._update_kwargs_with_deployment
    fn update_with_deployment(&self, request: &mut JsonObject, deployment: &DeploymentSpec);

    // the logging object's get_chat_completion_prompt
    async fn compile_prompt(
        &self,
        call: CompilePromptCall<'_>,
    ) -> Result<CompiledPrompt, Self::Error>;

    // Router.get_model_list(model_name=model), where none or an empty list means a direct call
    fn has_deployments(&self, model: &str) -> bool;

    async fn call_direct(&self, request: JsonObject) -> Result<Self::Response, Self::Error>;

    async fn call_with_fallbacks(&self, request: JsonObject)
    -> Result<Self::Response, Self::Error>;
}
