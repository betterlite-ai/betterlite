use std::time::Duration;

use betterlite_context::resource_config_selection::responses::{
    ResponsesModelFacts, select_responses_config,
};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_resources::request::ListInputItemsQuery;
use betterlite_openai::response_resources::response::TypedReply;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use crate::api::response_id_traversal::response_id::rewrite_typed_response;
use crate::api::responses_read_http::service::{LogContext, ReadOutcome};

use super::call_plan::{
    LifecycleOperation, ReadCallPlan, affinity, mode_flag, resolve, timeout_deadline,
};
use super::error::{LifecycleError, LifecycleFailure};
use super::executor::{ReadRequest, ResponsesReadExecutor};
use super::input::{LifecycleParams, ResponsesInputItemsCall, ResponsesRetrieveCall};

// the state and the pre-call environment the read logs through, borrowed for one call
pub struct LifecycleLog<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct ResponsesLifecycle<Executor> {
    pub executor: Executor,
    pub default_timeout: Duration,
    pub facts: ResponsesModelFacts,
    pub redactor: Redactor,
}

impl<Executor> ResponsesLifecycle<Executor>
where
    Executor: ResponsesReadExecutor,
{
    // the sync function the four SDK entry points share; the mode flag is popped here with `is True`
    pub async fn get_responses(
        &self,
        call: &ResponsesRetrieveCall,
        mut log: LifecycleLog<'_, '_>,
    ) -> Result<TypedReply, LifecycleFailure> {
        let plan = self.plan(
            &call.response_id,
            call.custom_llm_provider.as_deref(),
            LifecycleOperation::Retrieve,
        )?;
        self.log_call(&mut log, &plan, &call.params)?;
        let params = call.params.read_params();
        let timeout = timeout_deadline(call.timeout.as_ref(), self.default_timeout);
        let request = ReadRequest {
            tag: &plan.tag,
            response_id: &plan.response_id,
            params: &params,
            extra_headers: call.extra_headers.as_ref(),
            extra_body: call.extra_body.as_ref(),
            query: None,
            timeout,
            mode: mode_flag(call.aget_responses.as_ref()),
            log: LogContext {
                state: log.state,
                env: log.env,
            },
        };
        match self.executor.read(request).await {
            Ok(ReadOutcome::Response(mut reply)) => {
                self.rewrite(&mut reply.response, &plan, &call.params)?;
                Ok(*reply)
            }
            Ok(ReadOutcome::InputItems(_)) => {
                Err(self.failure(LifecycleError::UnexpectedInputItems, Some(&plan.provider)))
            }
            Err(error) => Err(self.failure(LifecycleError::Read(error), Some(&plan.provider))),
        }
    }

    // the wrapper decodes first, hands the sync function its decoded id and provider, and rewrites once more
    pub async fn aget_responses(
        &self,
        call: &ResponsesRetrieveCall,
        log: LifecycleLog<'_, '_>,
    ) -> Result<TypedReply, LifecycleFailure> {
        let resolved = resolve(&call.response_id, call.custom_llm_provider.as_deref());
        let mut inner = call.clone();
        inner.response_id = resolved.response_id;
        inner.custom_llm_provider = resolved.provider.clone();
        inner.aget_responses = Some(Json::Bool(true));
        let mut reply = self.get_responses(&inner, log).await?;
        // the id is already encoded by now, so the wrapper's second rewrite reads it back unchanged
        let metadata = affinity(inner.params.litellm_metadata.as_ref())
            .map_err(|error| self.failure(error, resolved.provider.as_deref()))?;
        rewrite_typed_response(&mut reply.response, resolved.provider.as_deref(), &metadata)
            .map_err(|error| {
                self.failure(LifecycleError::Rewrite(error), resolved.provider.as_deref())
            })?;
        Ok(reply)
    }

    pub async fn list_input_items(
        &self,
        call: &ResponsesInputItemsCall,
        mut log: LifecycleLog<'_, '_>,
    ) -> Result<Json, LifecycleFailure> {
        let plan = self.plan(
            &call.response_id,
            call.custom_llm_provider.as_deref(),
            LifecycleOperation::InputItems,
        )?;
        self.log_call(&mut log, &plan, &call.params)?;
        let query = ListInputItemsQuery {
            after: call.after.clone(),
            before: call.before.clone(),
            include: call.include.clone(),
            limit: call.limit,
            order: call.order,
        };
        let params = call.params.read_params();
        let timeout = timeout_deadline(call.timeout.as_ref(), self.default_timeout);
        let request = ReadRequest {
            tag: &plan.tag,
            response_id: &plan.response_id,
            params: &params,
            extra_headers: call.extra_headers.as_ref(),
            extra_body: None,
            query: Some(&query),
            timeout,
            mode: mode_flag(call.alist_input_items.as_ref()),
            log: LogContext {
                state: log.state,
                env: log.env,
            },
        };
        // the item list is the provider's own JSON: no id rewrite touches it
        match self.executor.read(request).await {
            Ok(ReadOutcome::InputItems(items)) => Ok(items),
            Ok(ReadOutcome::Response(_)) => {
                Err(self.failure(LifecycleError::UnexpectedResponse, Some(&plan.provider)))
            }
            Err(error) => Err(self.failure(LifecycleError::Read(error), Some(&plan.provider))),
        }
    }

    // the wrapper decodes and sets the mode flag, and the sync function does the rest
    pub async fn alist_input_items(
        &self,
        call: &ResponsesInputItemsCall,
        log: LifecycleLog<'_, '_>,
    ) -> Result<Json, LifecycleFailure> {
        let resolved = resolve(&call.response_id, call.custom_llm_provider.as_deref());
        let mut inner = call.clone();
        inner.response_id = resolved.response_id;
        inner.custom_llm_provider = resolved.provider;
        inner.alist_input_items = Some(Json::Bool(true));
        self.list_input_items(&inner, log).await
    }

    fn plan(
        &self,
        response_id: &str,
        explicit_provider: Option<&str>,
        operation: LifecycleOperation,
    ) -> Result<ReadCallPlan, LifecycleFailure> {
        let resolved = resolve(response_id, explicit_provider);
        let Some(provider) = resolved.provider else {
            return Err(self.failure(LifecycleError::MissingProvider, None));
        };
        let unsupported = match operation {
            LifecycleOperation::Retrieve => LifecycleError::UnsupportedRetrieve {
                provider: provider.clone(),
            },
            LifecycleOperation::InputItems => LifecycleError::UnsupportedInputItems {
                provider: provider.clone(),
            },
        };
        let Some(tag) = provider
            .parse::<ProviderId>()
            .ok()
            .and_then(|id| select_responses_config(Some(id), None, &self.facts))
        else {
            return Err(self.failure(unsupported, Some(&provider)));
        };
        Ok(ReadCallPlan {
            response_id: resolved.response_id,
            provider,
            tag,
        })
    }

    // update_from_kwargs(kwargs=local_vars, model=None, optional_params, litellm_params, custom_llm_provider)
    fn log_call(
        &self,
        log: &mut LifecycleLog<'_, '_>,
        plan: &ReadCallPlan,
        params: &LifecycleParams,
    ) -> Result<(), LifecycleFailure> {
        let mut kwargs = IndexMap::new();
        if let Some(metadata) = &params.metadata {
            kwargs.insert(String::from("metadata"), Json::Object(metadata.clone()));
        }
        if let Some(litellm_metadata) = &params.litellm_metadata {
            kwargs.insert(
                String::from("litellm_metadata"),
                Json::Object(litellm_metadata.clone()),
            );
        }
        let mut litellm_params = IndexMap::new();
        litellm_params.insert(
            String::from("litellm_call_id"),
            params
                .litellm_call_id
                .as_ref()
                .map_or(Json::Null, |id| Json::Text(id.clone())),
        );
        let mut optional_params = IndexMap::new();
        optional_params.insert(
            String::from("response_id"),
            Json::Text(plan.response_id.clone()),
        );
        let mut additional_params = IndexMap::new();
        additional_params.insert(
            String::from("custom_llm_provider"),
            Json::Text(plan.provider.clone()),
        );
        update_from_kwargs(
            &mut *log.state,
            &kwargs,
            Some(&litellm_params),
            CallUpdate {
                model: None,
                user: None,
                optional_params,
                additional_params,
            },
        )
        .map_err(|error| self.failure(LifecycleError::Logging(error), Some(&plan.provider)))
    }

    fn rewrite(
        &self,
        response: &mut ResponsesApiResponse,
        plan: &ReadCallPlan,
        params: &LifecycleParams,
    ) -> Result<(), LifecycleFailure> {
        let metadata = affinity(params.litellm_metadata.as_ref())
            .map_err(|error| self.failure(error, Some(&plan.provider)))?;
        rewrite_typed_response(response, Some(&plan.provider), &metadata)
            .map_err(|error| self.failure(LifecycleError::Rewrite(error), Some(&plan.provider)))
    }

    fn failure(&self, error: LifecycleError, provider: Option<&str>) -> LifecycleFailure {
        LifecycleFailure::new(error, provider.unwrap_or_default(), &self.redactor)
    }
}
