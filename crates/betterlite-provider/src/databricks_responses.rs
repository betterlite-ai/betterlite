use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_input::transform_responses_request;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::databricks_auth::Headers;
use crate::databricks_auth::api_base::get_api_base;
use crate::databricks_auth::endpoint::EndpointType;
use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::error::DatabricksError;
use crate::databricks_auth::oauth::OauthTokenSource;
use crate::databricks_auth::validate::{ValidationRequest, validate_environment};

pub const PROVIDER_PREFIX: &str = "databricks/";

pub struct DatabricksResponsesAPIConfig;

pub struct ResponsesHeaders<'value> {
    pub api_key: Option<&'value SecretString>,
    pub api_base: Option<&'value str>,
    pub headers: Option<Headers>,
}

impl DatabricksResponsesAPIConfig {
    #[must_use]
    pub const fn provider(&self) -> ProviderId {
        // LiteLLM's custom_llm_provider property answers DATABRICKS for this config
        ProviderId::Databricks
    }

    #[must_use]
    pub const fn supports_native_websocket(&self) -> bool {
        // Databricks does not support native WebSocket for the Responses API
        false
    }

    pub async fn headers(
        &self,
        request: ResponsesHeaders<'_>,
        env: &DatabricksEnv,
        tokens: &dyn OauthTokenSource,
    ) -> Result<Headers, DatabricksError> {
        let (_, mut headers) = validate_environment(
            ValidationRequest {
                api_key: request.api_key,
                api_base: request.api_base,
                endpoint: EndpointType::ChatCompletions,
                custom_endpoint: false,
                headers: request.headers,
                custom_user_agent: None,
            },
            env,
            tokens,
        )
        .await?;
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        Ok(headers)
    }

    pub fn complete_url(
        &self,
        api_base: Option<&str>,
        env: &DatabricksEnv,
    ) -> Result<String, DatabricksError> {
        let candidate = api_base.map(str::to_owned).or_else(|| env.api_base.clone());
        let base = get_api_base(candidate.as_deref(), false)?;
        Ok(format!("{}/responses", base.trim_end_matches('/')))
    }

    pub fn create_request(
        &self,
        model: &str,
        input: &ResponsesInput,
        optional_params: &Object,
        model_info: Option<&Object>,
    ) -> Result<Object, ResponseInputError> {
        let name = model.strip_prefix(PROVIDER_PREFIX).unwrap_or(model);
        transform_responses_request(
            name,
            input,
            optional_params,
            ProviderId::Databricks,
            model_info,
        )
    }
}

mod tests;
