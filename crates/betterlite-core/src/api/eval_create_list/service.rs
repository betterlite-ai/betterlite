use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_context::resource_config_selection::skills_evals::select_evals_config;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::evals::transformation::EvalsRequestContext;
use betterlite_openai::evals::types::{Eval, ListEvalsResponse};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::call::{
    CreateEvalCall, EvalCreation, EvalListing, EvalsCallParams, ListEvalsCall, resolved_timeout,
};
use super::config::EvalsConfigLookup;
use super::error::{EvalsCreateListError, EvalsCreateListFailureError, failure};
use super::handler::{CreateEvalHttpCall, EvalsCreateListHandler, ListEvalsHttpCall};
use super::logging::log_pre_call;
use super::request::{create_request, list_params, merge_extra_body};
use super::{DEFAULT_EVALS_PROVIDER, DEFAULT_OPENAI_API_BASE};
use crate::api::eval_http::exchange::{EvalConfig, EvalConnection, EvalLogging};
use crate::api::passthrough_call::logging::json_object;

// the collaborators the operations call: B9370's handler, the provider configs, the pooled client and B247's logging
pub struct EvalsCreateListService<'deps, Handler, Configs> {
    pub handler: &'deps Handler,
    pub configs: &'deps Configs,
    pub client: &'deps ProxiedClient,
    pub core: &'deps CoreSettings,
    pub redactor: &'deps Redactor,
}

// what both operations resolve before they build their request: the provider, its config and validated headers
struct ProviderCall<'deps> {
    provider: String,
    config: EvalConfig<'deps>,
    headers: Headers,
    litellm_params: Object,
    timeout: Duration,
}

impl ProviderCall<'_> {
    // the provider call's own fields as the transform context, borrowed for the transform's lifetime
    const fn context(&self) -> EvalsRequestContext<'_> {
        EvalsRequestContext {
            litellm_params: &self.litellm_params,
            headers: &self.headers,
        }
    }
}

// which operation asks, for the ValueError LiteLLM raises when the manager answers None
#[derive(Clone, Copy)]
enum Operation {
    Create,
    List,
}

impl Operation {
    fn unsupported(self, provider: &str) -> EvalsCreateListError {
        match self {
            Self::Create => EvalsCreateListError::CreateUnsupported {
                provider: provider.to_owned(),
            },
            Self::List => EvalsCreateListError::ListUnsupported {
                provider: provider.to_owned(),
            },
        }
    }
}

// LiteLLM spells the blocking and async form of each operation out twice; one async body serves both names
macro_rules! create_entries {
    ($($name:ident),+ $(,)?) => { $(
        pub async fn $name(
            &self,
            creation: EvalCreation<'_, '_>,
        ) -> Result<Eval, EvalsCreateListFailureError> {
            let EvalCreation { call, logging, env } = creation;
            self.create(call, logging, env)
                .await
                .map_err(|error| failure(error, &call.params, self.redactor))
        }
    )+ };
}

macro_rules! list_entries {
    ($($name:ident),+ $(,)?) => { $(
        pub async fn $name(
            &self,
            listing: EvalListing<'_, '_>,
        ) -> Result<ListEvalsResponse, EvalsCreateListFailureError> {
            let EvalListing { call, logging, env } = listing;
            self.list(call, logging, env)
                .await
                .map_err(|error| failure(error, &call.params, self.redactor))
        }
    )+ };
}

impl<'deps, Handler, Configs> EvalsCreateListService<'deps, Handler, Configs>
where
    Handler: EvalsCreateListHandler,
    Configs: EvalsConfigLookup,
{
    create_entries!(create_eval, acreate_eval);
    list_entries!(list_evals, alist_evals);

    async fn create(
        &self,
        call: &CreateEvalCall<'_>,
        logging: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<Eval, EvalsCreateListError> {
        let request = create_request(call)?;
        let prepared = self.prepare(&call.params, Operation::Create)?;
        let context = prepared.context();
        let mut body = prepared
            .config
            .transform_create_eval_request(&request, &context)?;
        merge_extra_body(&mut body, call.extra_body)?;
        let url = prepared.config.get_complete_url(
            Some(api_base(&prepared.litellm_params)),
            "evals",
            None,
        )?;
        log_pre_call(
            logging,
            &call.params,
            &prepared.provider,
            json_object(&body),
        )?;
        self.handler
            .create_eval(CreateEvalHttpCall {
                connection: connection(prepared.config, self.client, &prepared.headers, &url),
                logging: EvalLogging {
                    state: logging,
                    env,
                },
                timeout: prepared.timeout,
                body: &body,
            })
            .await
            .map_err(EvalsCreateListError::from)
    }

    async fn list(
        &self,
        call: &ListEvalsCall<'_>,
        logging: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ListEvalsResponse, EvalsCreateListError> {
        let params = list_params(call)?;
        let prepared = self.prepare(&call.params, Operation::List)?;
        let context = prepared.context();
        let (url, query) = prepared
            .config
            .transform_list_evals_request(&params, &context)?;
        log_pre_call(
            logging,
            &call.params,
            &prepared.provider,
            json_object(&query),
        )?;
        self.handler
            .list_evals(ListEvalsHttpCall {
                connection: connection(prepared.config, self.client, &prepared.headers, &url),
                logging: EvalLogging {
                    state: logging,
                    env,
                },
                timeout: prepared.timeout,
                query: &query,
            })
            .await
            .map_err(EvalsCreateListError::from)
    }

    fn prepare(
        &self,
        params: &EvalsCallParams<'_>,
        operation: Operation,
    ) -> Result<ProviderCall<'deps>, EvalsCreateListError> {
        let provider = params.custom_llm_provider.unwrap_or(DEFAULT_EVALS_PROVIDER);
        let kind = provider.parse::<ProviderId>().map_err(|_source| {
            EvalsCreateListError::UnknownProvider {
                provider: provider.to_owned(),
            }
        })?;
        let tag = select_evals_config(kind).ok_or_else(|| operation.unsupported(provider))?;
        let config = self
            .configs
            .evals_config(tag)
            .ok_or_else(|| operation.unsupported(provider))?;
        let litellm_params = litellm_params_object(params.kwargs)?;
        let extra_headers = params.extra_headers.cloned().unwrap_or_default();
        let headers = config.validate_environment(extra_headers, Some(&litellm_params))?;
        let timeout = resolved_timeout(params.timeout, self.core)?;
        Ok(ProviderCall {
            provider: provider.to_owned(),
            config,
            headers,
            litellm_params,
            timeout,
        })
    }
}

// the connection both operations hand the handler, so the provider config, client, headers and url share one borrow
fn connection<'call>(
    config: EvalConfig<'call>,
    client: &'call ProxiedClient,
    headers: &'call Headers,
    url: &'call str,
) -> EvalConnection<'call> {
    EvalConnection {
        config,
        client,
        url,
        headers,
    }
}

// litellm_params.api_base or DEFAULT_OPENAI_API_BASE, where an empty string counts as unset
fn api_base(litellm_params: &Object) -> &str {
    litellm_params
        .get("api_base")
        .and_then(|value| value.as_str())
        .filter(|text| !text.is_empty())
        .unwrap_or(DEFAULT_OPENAI_API_BASE)
}

// GenericLiteLLMParams(**kwargs): the provider transforms read the caller's keywords as one object
fn litellm_params_object(kwargs: &IndexMap<String, Json>) -> Result<Object, EvalsCreateListError> {
    serde_json::to_value(kwargs)
        .and_then(serde_json::from_value::<Object>)
        .map_err(EvalsCreateListError::from)
}
