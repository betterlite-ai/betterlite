use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::model_budget_limiter::enforcement::BudgetSubject;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::drop_params::DeploymentDropPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::messages_context_clear::counter::{TokenCountRequest, TokenCounter};
use betterlite_core::api::messages_context_clear::error::ClearToolUsesError;
use betterlite_core::api::summary_history::count::{CountedInput, HistoryTokenCounter};
use betterlite_core::api::summary_history::error::SummaryHistoryError;
use betterlite_core::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use betterlite_core::api::summary_model_access::error::PolicyFailure;
use betterlite_core::api::summary_model_access::repositories::SummaryAccessRepositories;
use betterlite_core::api::summary_model_limits::budget::SummaryModelBudgets;
use betterlite_core::api::summary_model_limits::error::LimiterFailure;
use betterlite_core::api::summary_model_limits::fields::BudgetMap;
use betterlite_core::api::summary_model_limits::rate::{
    BaseDescriptorRequest, DescriptorScope, LimiterCapability, ProxyHooks, ReadOnlyCheck,
    SummaryDescriptor, SummaryRateLimiter,
};
use betterlite_core::mock::chat::response as mock_chat;
use betterlite_core::mock::embedding::response as mock_embedding;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_error::error::ErrorClass;
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

use super::super::error::SdkHttpError;

const MODEL_ID: &str = "summary";

// the real chat executor behind the direct summary call, pointed at the case's loopback provider
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

// the deployment the summary request is sent through, with the case's loopback api_base
pub(super) fn deployment(api_base: &str) -> Result<ChatDeployment, SdkHttpError> {
    let endpoint = Url::parse(&format!("{api_base}/chat/completions"))
        .map_err(|error| SdkHttpError::Url(Box::new(error)))?;
    Ok(ChatDeployment {
        model_group: String::from("main-model"),
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

// both editors' token counts answer the case's one number, as one scripted litellm.token_counter does
pub(super) struct FixedCounter {
    pub(super) count: usize,
}

impl TokenCounter for FixedCounter {
    fn count(&self, _request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError> {
        Ok(self.count)
    }
}

impl HistoryTokenCounter for FixedCounter {
    fn count(
        &self,
        _model: &str,
        _input: CountedInput<'_>,
    ) -> impl Future<Output = Result<usize, SummaryHistoryError>> + Send {
        ready(Ok(self.count))
    }
}

// the policy seams never run while the principal is absent, but the editor still names their types
pub(super) struct NoChecks;

type Access<'router> = Option<&'router dyn ModelAccessView>;
type UserRow<'row> = &'row LiteLLMUserTable;
type ProjectRow<'row> = &'row ProjectTable;
type Check = Result<(), PolicyFailure<io::Error>>;

impl SummaryModelChecks for NoChecks {
    type Failure = io::Error;

    fn object_call(&self, _router: Access<'_>, _call: &ObjectCall<'_>) -> Check {
        Ok(())
    }

    fn user_call(&self, _router: Access<'_>, _model: &str, _user: UserRow<'_>) -> Check {
        Ok(())
    }

    fn project_call(&self, _router: Access<'_>, _model: &str, _project: ProjectRow<'_>) -> Check {
        Ok(())
    }
}

pub(super) struct NoRows;

impl SummaryAccessRepositories for NoRows {
    type Failure = io::Error;

    fn user(
        &self,
        _user_id: &str,
        _user_id_upsert: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, io::Error>> + Send {
        ready(Ok(None))
    }

    fn project(
        &self,
        _project_id: &str,
    ) -> impl Future<Output = Result<Option<ProjectTable>, io::Error>> + Send {
        ready(Ok(None))
    }

    fn membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMTeamMembership>, io::Error>> + Send {
        ready(Ok(None))
    }
}

pub(super) struct NoBudgets;

impl SummaryModelBudgets for NoBudgets {
    fn within_model_budget(
        &self,
        _subject: &BudgetSubject<'_>,
        _model_max_budget: &BudgetMap,
        _model: &str,
    ) -> impl Future<Output = Result<(), ErrorClass>> + Send {
        ready(Ok(()))
    }
}

pub(super) struct NoLimiter;

impl SummaryRateLimiter for NoLimiter {
    type Failure = io::Error;

    fn supports(&self, _capability: LimiterCapability) -> bool {
        false
    }

    fn create_rate_limit_descriptors(
        &self,
        _request: &BaseDescriptorRequest<'_>,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<io::Error>> {
        Ok(Vec::new())
    }

    fn add_scope_descriptors(
        &self,
        _scope: DescriptorScope,
        _principal: &UserAPIKeyAuth,
        _requested_model: &str,
        _descriptors: &mut Vec<SummaryDescriptor>,
    ) -> Result<(), LimiterFailure<io::Error>> {
        Ok(())
    }

    fn should_rate_limit(
        &self,
        _check: &ReadOnlyCheck<'_>,
    ) -> impl Future<Output = Result<Option<String>, LimiterFailure<io::Error>>> + Send {
        ready(Ok(None))
    }
}

pub(super) struct NoHooks;

impl ProxyHooks for NoHooks {
    type Limiter = NoLimiter;
    type Error = io::Error;

    fn get_proxy_hook(&self, _hook: &str) -> Result<Option<&NoLimiter>, io::Error> {
        Ok(None)
    }
}
