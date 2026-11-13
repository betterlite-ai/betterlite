use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use secrecy::SecretString;
use serde_json::Number;

use crate::vector_stores::embedding::QueryEmbeddingExecutor;
use crate::vector_stores::environment::Params;
use crate::vector_stores::error::EmbeddingCallError;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Call {
    pub(super) model: String,
    pub(super) query: String,
    pub(super) configuration: Params,
}

pub(super) struct FixtureExecutor {
    vector: Vec<f64>,
    calls: Mutex<Vec<Call>>,
}

impl FixtureExecutor {
    pub(super) fn new(vector: Vec<f64>) -> Self {
        Self {
            vector,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("fixture lock").clone()
    }

    fn record(&self, model: &str, query: &str, configuration: &Params) {
        self.calls.lock().expect("fixture lock").push(Call {
            model: model.to_owned(),
            query: query.to_owned(),
            configuration: configuration.clone(),
        });
    }

    fn response(&self) -> EmbeddingResponse {
        let numbers = self
            .vector
            .iter()
            .map(|value| Number::from_f64(*value).expect("finite fixture vector"))
            .collect();
        EmbeddingResponse {
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
        }
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
        self.record(model, query, configuration);
        Ok(self.response())
    }

    async fn aembed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, EmbeddingCallError> {
        self.record(model, query, configuration);
        Ok(self.response())
    }
}

pub(super) struct NoSources;

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

pub(super) fn empty_credentials() -> DeploymentSettings {
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
