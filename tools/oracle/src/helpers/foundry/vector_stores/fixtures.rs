use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::vector_stores::embedding::QueryEmbeddingExecutor;
use betterlite_foundry::vector_stores::environment::Params;
use betterlite_foundry::vector_stores::error::EmbeddingCallError;
use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use secrecy::SecretString;
use serde::Serialize;
use serde_json::Number;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CallRecord {
    pub(crate) model: String,
    pub(crate) query: String,
    pub(crate) configuration: Params,
}

pub(crate) struct FixtureExecutor {
    vectors: Vec<Vec<f64>>,
    calls: Mutex<Vec<CallRecord>>,
}

impl FixtureExecutor {
    pub(crate) fn new(vectors: Vec<Vec<f64>>) -> Self {
        Self {
            vectors,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<CallRecord> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn respond(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, EmbeddingCallError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let index = calls.len();
        calls.push(CallRecord {
            model: model.to_owned(),
            query: query.to_owned(),
            configuration: configuration.clone(),
        });
        let selected = self
            .vectors
            .get(index)
            .or_else(|| self.vectors.last())
            .cloned()
            .unwrap_or_default();
        let mut numbers = Vec::with_capacity(selected.len());
        for value in &selected {
            let Some(number) = Number::from_f64(*value) else {
                return Err(EmbeddingCallError::Provider {
                    message: "the fixture vector is not finite".to_owned(),
                });
            };
            numbers.push(number);
        }
        Ok(EmbeddingResponse {
            object: "list".to_owned(),
            data: vec![EmbeddingRecord {
                object: "embedding".to_owned(),
                embedding: EmbeddingVector::Floats(numbers),
                index: 0,
            }],
            model: "fixture-model".to_owned(),
            usage: EmbeddingUsage {
                prompt_tokens: 1,
                completion_tokens: 0,
                completion_tokens_details: None,
                prompt_tokens_details: None,
                total_tokens: 1,
            },
        })
    }
}

#[async_trait]
impl QueryEmbeddingExecutor for FixtureExecutor {
    fn embed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, EmbeddingCallError> {
        self.respond(model, query, configuration)
    }

    async fn aembed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, EmbeddingCallError> {
        self.respond(model, query, configuration)
    }
}

pub(crate) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}

pub(crate) fn empty_credentials() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::new(),
        azure_username: None,
        azure_password: None,
    }
}
