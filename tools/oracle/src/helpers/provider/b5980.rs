use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::databricks_auth::endpoint::EndpointType;
use betterlite_provider::databricks_auth::environment::DatabricksEnv;
use betterlite_provider::databricks_auth::oauth::OauthReply;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

mod cost;
mod ops;

pub(crate) const DEFAULT_VERSION: &str = "1.105.1";

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Validate(ValidateInput),
    ApiBase {
        cases: Vec<ApiBaseCase>,
    },
    UserAgent {
        cases: Vec<UserAgentCase>,
    },
    Redaction {
        headers: BTreeMap<String, String>,
        data: Json,
    },
    Cost {
        cases: Vec<CostCase>,
    },
    Endpoints {
        #[serde(default)]
        env: EnvInput,
        embedding: Vec<EmbeddingCase>,
        responses: Vec<UrlCase>,
    },
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub(super) struct EnvInput {
    pub(super) client_id: Option<String>,
    pub(super) client_secret: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) user_agent_override: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub(super) struct OauthScript {
    pub(super) status: Option<u16>,
    pub(super) body: Option<String>,
    pub(super) transport: Option<String>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EndpointChoice {
    #[default]
    ChatCompletions,
    Embeddings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidateInput {
    #[serde(default)]
    pub(super) env: EnvInput,
    pub(super) cases: Vec<ValidateCase>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub(super) struct ValidateCase {
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) endpoint: EndpointChoice,
    pub(super) custom_endpoint: bool,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) custom_user_agent: Option<String>,
    pub(super) oauth: OauthScript,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApiBaseCase {
    pub(super) api_base: Option<String>,
    pub(super) use_ai_gateway: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UserAgentCase {
    pub(super) custom: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EmbeddingCase {
    pub(super) model: String,
    pub(super) input: Json,
    #[serde(default)]
    pub(super) optional_params: Object,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) custom_endpoint: bool,
    #[serde(default)]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) env: EnvInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UrlCase {
    pub(super) api_base: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostCase {
    pub(super) models: Vec<String>,
    #[serde(default)]
    pub(super) service_tier: Option<String>,
    #[serde(default)]
    pub(super) pricing: BTreeMap<String, RateCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RateCase {
    pub(super) input_cost_per_token: String,
    pub(super) output_cost_per_token: String,
}

impl EnvInput {
    pub(super) fn build(&self) -> DatabricksEnv {
        DatabricksEnv {
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.as_deref().map(SecretString::from),
            api_base: self.api_base.clone(),
            user_agent_override: self.user_agent_override.clone(),
            version: DEFAULT_VERSION.to_owned(),
        }
    }
}

impl OauthScript {
    pub(super) fn reply(&self) -> OauthReply {
        if let Some(error) = &self.transport {
            return OauthReply::Transport(error.clone());
        }
        OauthReply::Reply {
            status: self.status.unwrap_or(200),
            body: SecretString::from(self.body.clone().unwrap_or_default()),
        }
    }
}

impl EndpointChoice {
    pub(super) const fn endpoint(self) -> EndpointType {
        match self {
            Self::ChatCompletions => EndpointType::ChatCompletions,
            Self::Embeddings => EndpointType::Embeddings,
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Validate(validate) => ops::validate(validate),
            Input::ApiBase { cases } => Ok(json!(
                cases
                    .iter()
                    .map(ops::api_base_case)
                    .collect::<Vec<serde_json::Value>>()
            )),
            Input::UserAgent { cases } => Ok(ops::user_agent(cases)),
            Input::Redaction { headers, data } => Ok(ops::redaction(headers, data)),
            Input::Cost { cases } => cost::cost(cases),
            Input::Endpoints {
                env,
                embedding,
                responses,
            } => ops::endpoints(env, embedding, responses),
        }
    }
}
