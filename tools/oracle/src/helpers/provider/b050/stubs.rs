use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_provider::a2a_chat::auth::EntraResolver;
use betterlite_provider::a2a_chat::registry::{A2aAgentLookup, RegisteredA2aAgent};
use betterlite_provider::a2a_chat::reply as a2a_reply;
use betterlite_provider::langflow_chat::reply as langflow_reply;
use betterlite_provider::langgraph_chat::reply as langgraph_reply;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::count::error::CountError;
use secrecy::SecretString;

use super::script::AgentCase;

struct NoTraffic;

impl OidcHttp for NoTraffic {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the dispatch helper reads no OIDC document"),
        })
    }
}

impl AzureOidcTokens for NoTraffic {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the dispatch helper mints no Azure token"),
        })
    }
}

impl OidcClock for NoTraffic {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoTraffic),
        azure_tokens: Arc::new(NoTraffic),
        clock: Arc::new(NoTraffic),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) struct NoEntra;

impl EntraResolver for NoEntra {
    fn token(&self, _params: &Object) -> Result<SecretString, CredentialsError> {
        Err(CredentialsError::ProviderFailed {
            message: String::from("the dispatch helper holds no Azure AD tenant"),
        })
    }

    fn auth_header(&self, _params: &Object) -> Result<BTreeMap<String, String>, CredentialsError> {
        Err(CredentialsError::ProviderFailed {
            message: String::from("the dispatch helper holds no hop header"),
        })
    }
}

pub(super) struct Registry {
    agents: BTreeMap<String, RegisteredA2aAgent>,
}

impl Registry {
    pub(super) fn new(cases: &[AgentCase]) -> Self {
        let mut agents = BTreeMap::new();
        for case in cases {
            let agent = RegisteredA2aAgent {
                agent_card_params: case.agent_card_params.clone(),
                litellm_params: case.litellm_params.clone(),
            };
            agents.insert(case.agent_name.clone(), agent);
        }
        Self { agents }
    }
}

impl A2aAgentLookup for Registry {
    fn agent_by_name(&self, name: &str) -> Option<RegisteredA2aAgent> {
        self.agents.get(name).cloned()
    }
}

// the reply carries no usage and no ids into the compared answer, so every count is one
pub(super) struct Meter;

#[async_trait]
impl a2a_reply::TokenCounter for Meter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

#[async_trait]
impl langgraph_reply::TokenCounter for Meter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

#[async_trait]
impl langflow_reply::TokenCounter for Meter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}
