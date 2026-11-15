use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::databricks_auth::Headers;
use crate::databricks_auth::endpoint::EndpointType;
use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::error::DatabricksError;
use crate::databricks_auth::oauth::OauthTokenSource;
use crate::databricks_auth::validate::{ValidationRequest, validate_environment};

// LiteLLM's DatabricksEmbeddingConfig carries the instruction on the value, since Rust has no class state
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabricksEmbeddingConfig {
    pub instruction: Option<String>,
}

pub struct EmbeddingPreparation<'value> {
    pub optional_params: Object,
    pub api_key: Option<&'value SecretString>,
    pub api_base: Option<&'value str>,
    pub custom_endpoint: bool,
    pub headers: Option<Headers>,
}

pub struct PreparedEmbedding {
    pub url: String,
    pub headers: Headers,
    pub optional_params: Object,
}

impl DatabricksEmbeddingConfig {
    #[must_use]
    pub fn get_config(&self) -> BTreeMap<String, String> {
        let mut config = BTreeMap::new();
        if let Some(instruction) = &self.instruction {
            config.insert("instruction".to_owned(), instruction.clone());
        }
        config
    }

    #[must_use]
    #[cfg_attr(test, mutants::skip)]
    pub const fn get_supported_openai_params(&self) -> Vec<String> {
        // LiteLLM's Databricks config supports no optional OpenAI embedding parameter
        Vec::new()
    }

    #[must_use]
    pub const fn map_openai_params(&self, optional_params: Object) -> Object {
        // the source hands optional_params back untouched, so non-default params never reach the body
        optional_params
    }
}

fn pop_user_agent(
    optional_params: &mut Object,
    name: &str,
) -> Result<Option<String>, DatabricksError> {
    let Some(value) = optional_params.shift_remove(name) else {
        return Ok(None);
    };
    let text = match value {
        Json::Null => None,
        Json::Text(text) => Some(text),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(DatabricksError {
                status_code: 400,
                message: format!("the {name} optional parameter must be a string"),
                headers: None,
            });
        }
    };
    Ok(text.filter(|agent| !agent.is_empty()))
}

fn pop_user_agent_parameter(
    optional_params: &mut Object,
) -> Result<Option<String>, DatabricksError> {
    if let Some(user_agent) = pop_user_agent(optional_params, "user_agent")? {
        return Ok(Some(user_agent));
    }
    // the Python "or" only pops databricks_user_agent when user_agent is absent or empty
    pop_user_agent(optional_params, "databricks_user_agent")
}

pub async fn prepare_embedding(
    preparation: EmbeddingPreparation<'_>,
    env: &DatabricksEnv,
    tokens: &dyn OauthTokenSource,
) -> Result<PreparedEmbedding, DatabricksError> {
    let mut optional_params = preparation.optional_params;
    let custom_user_agent =
        pop_user_agent_parameter(&mut optional_params)?.or_else(|| env.user_agent_override.clone());
    let (url, headers) = validate_environment(
        ValidationRequest {
            api_key: preparation.api_key,
            api_base: preparation.api_base,
            endpoint: EndpointType::Embeddings,
            custom_endpoint: preparation.custom_endpoint,
            headers: preparation.headers,
            custom_user_agent: custom_user_agent.as_deref(),
        },
        env,
        tokens,
    )
    .await?;
    Ok(PreparedEmbedding {
        url,
        headers,
        optional_params,
    })
}

mod tests;
