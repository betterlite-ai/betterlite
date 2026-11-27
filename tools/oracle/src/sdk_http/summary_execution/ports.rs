use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::drop_params::DeploymentDropPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::{ChatDeployment, Provider};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use secrecy::SecretString;
use url::Url;
use uuid::Uuid;

use betterlite_core::api::summary_execution::completion::{
    ExecutorCompletion, SummaryCall, SummaryCallFailure, SummaryCompletion,
};
use betterlite_core::api::summary_execution::extract::CompletionAnswer;
use betterlite_core::mock::chat::response as mock_chat;
use betterlite_core::mock::embedding::response as mock_embedding;

use super::super::error::SdkHttpError;
use super::{CallRecord, Route};

const MODEL_ID: &str = "summary";

// the workspace's own chat executor, pointed at the case's loopback provider through one deployment
pub(super) fn executor() -> Result<ChatExecutor, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    Ok(ChatExecutor {
        client,
        deployments: Vec::new(),
        catalog: CatalogCapabilities {
            // litellm.open_ai_chat_completion_models names gpt-4o-mini, so the user param is supported
            chat_models: BTreeSet::from([String::from("gpt-4o-mini")]),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(7)),
        clock: || 1_700_000_000,
        next_id: || Uuid::from_u128(7),
        mock: MockSettings::default(),
        mock_chat,
        mock_embedding,
        redactor,
        bundled_catalog: Arc::default(),
    })
}

// the model alias the router resolves, with the parent key the provider sees
pub(super) fn deployment(api_base: &str) -> Result<ChatDeployment, SdkHttpError> {
    let endpoint = Url::parse(&format!("{api_base}/chat/completions"))
        .map_err(|error| SdkHttpError::Url(Box::new(error)))?;
    Ok(ChatDeployment {
        model_group: String::from("sum"),
        litellm_model: String::from("openai/gpt-4o-mini"),
        provider: Provider::OpenAi {
            model: String::from("gpt-4o-mini"),
        },
        api_base: api_base.to_owned(),
        endpoint,
        authorization: None,
        api_key: Some(SecretString::from("sk-test")),
        organization: None,
        timeout: Duration::from_secs(60),
        timeout_seconds: 60.0_f64,
        model_id: String::from(MODEL_ID),
        weight: 0,
        rpm: 0,
        tpm: 0,
        global_drop_params: false,
        drop_policy: DeploymentDropPolicy::default(),
    })
}

// the routed interface or the direct pipeline, both delegating to the real executor
pub(super) struct Port<'port> {
    route: Route,
    executor: &'port ChatExecutor,
    deployment: &'port ChatDeployment,
    calls: Mutex<Vec<CallRecord>>,
}

impl<'port> Port<'port> {
    pub(super) fn new(
        route: Route,
        executor: &'port ChatExecutor,
        deployment: &'port ChatDeployment,
    ) -> Self {
        Self {
            route,
            executor,
            deployment,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn records(&self) -> Vec<CallRecord> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: &SummaryCall<'_>) {
        let timeout = Duration::try_from_secs_f64(call.timeout_seconds)
            .map_or(0, |duration| duration.as_secs());
        let record = CallRecord {
            route: self.route,
            model: call.model.to_owned(),
            messages: call.messages.to_vec(),
            max_tokens: call.max_tokens,
            timeout_seconds: timeout,
            metadata: call.metadata.clone(),
            user: call.user.map(str::to_owned),
            region: call.allowed_model_region.map(str::to_owned),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

#[async_trait]
impl SummaryCompletion for Port<'_> {
    async fn complete(
        &self,
        call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        self.record(&call);
        let completion = ExecutorCompletion {
            executor: self.executor,
            deployment: self.deployment,
        };
        completion.complete(call).await
    }
}
