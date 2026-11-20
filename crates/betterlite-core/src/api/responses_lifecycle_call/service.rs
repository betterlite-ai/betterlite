use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::resource_config_selection::responses::ResponsesModelFacts;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_resources::request::ListInputItemsQuery;
use betterlite_openai::response_resources::response::TypedReply;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use crate::api::response_id_traversal::response_id::rewrite_typed_response;
use crate::api::responses_read_http::config::ReadParams;
use crate::api::responses_read_http::service::{ClientKind, LogContext, ReadOutcome};

use super::call_plan::{
    ReadCallPlan, affinity, decode, log_call, mode_flag, select, timeout_deadline,
};
use super::error::{LifecycleError, LifecycleFailure};
use super::executor::{ReadRequest, ResponsesReadExecutor};
use super::input::{
    ReadExtras, ReadIdentity, ResponsesInputItemsCall, ResponsesRetrieveCall, SdkRead,
};

pub struct LifecycleLog<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: PrePostEnv<'env>,
}

pub struct ResponsesLifecycle<Executor> {
    pub executor: Executor,
    pub default_timeout: Duration,
    pub facts: ResponsesModelFacts,
    pub redactor: Redactor,
}

struct ReadSpec<'call> {
    extra_headers: Option<&'call BTreeMap<String, String>>,
    extra_body: Option<&'call IndexMap<String, Json>>,
    query: Option<&'call ListInputItemsQuery>,
    timeout: Option<Duration>,
    mode: ClientKind,
}

impl<Executor> ResponsesLifecycle<Executor>
where
    Executor: ResponsesReadExecutor,
{
    // the sync function the four SDK entry points share; the mode flag is popped here with `is True`
    pub async fn get_responses(
        &self,
        call: &ResponsesRetrieveCall,
        log: LifecycleLog<'_, '_>,
    ) -> Result<TypedReply, Box<LifecycleFailure>> {
        let extras = ReadExtras::Retrieve {
            extra_body: call.extra_body.as_ref(),
            flag: call.aget_responses.as_ref(),
        };
        let (outcome, plan) = self.run_read(call.identity.sdk_read(extras), log).await?;
        self.retrieved_reply(outcome, &plan, &call.identity)
    }

    // the wrapper decodes first, hands the sync function its decoded id and provider, and rewrites once more
    pub async fn aget_responses(
        &self,
        call: &ResponsesRetrieveCall,
        log: LifecycleLog<'_, '_>,
    ) -> Result<TypedReply, Box<LifecycleFailure>> {
        let mut inner = call.clone();
        inner.identity = decode(&call.identity);
        inner.aget_responses = Some(Json::Bool(true));
        let mut reply = self.get_responses(&inner, log).await?;
        // the id is already encoded by now, so the wrapper's second rewrite reads it back unchanged
        let provider = inner.identity.custom_llm_provider.as_deref();
        self.rewrite(&mut reply.response, provider, &inner.identity)?;
        Ok(reply)
    }

    pub async fn list_input_items(
        &self,
        call: &ResponsesInputItemsCall,
        log: LifecycleLog<'_, '_>,
    ) -> Result<Json, Box<LifecycleFailure>> {
        let query = call.query();
        let extras = ReadExtras::InputItems {
            query: &query,
            flag: call.alist_input_items.as_ref(),
        };
        let read = call.identity.sdk_read(extras);
        let (outcome, plan) = self.run_read(read, log).await?;
        self.item_page(outcome, &plan.provider)
    }

    // the wrapper decodes and sets the mode flag, and the sync function does the rest
    pub async fn alist_input_items(
        &self,
        mut call: ResponsesInputItemsCall,
        log: LifecycleLog<'_, '_>,
    ) -> Result<Json, Box<LifecycleFailure>> {
        call.identity = decode(&call.identity);
        call.alist_input_items = Some(Json::Bool(true));
        self.list_input_items(&call, log).await
    }

    // the shared sync path: resolve the plan, log the pre-call, build the request and run it
    async fn run_read(
        &self,
        read: SdkRead<'_>,
        mut log: LifecycleLog<'_, '_>,
    ) -> Result<(ReadOutcome, ReadCallPlan), Box<LifecycleFailure>> {
        let operation = read.extras.operation();
        let (extra_body, query, flag) = match read.extras {
            ReadExtras::Retrieve { extra_body, flag } => (extra_body, None, flag),
            ReadExtras::InputItems { query, flag } => (None, Some(query), flag),
        };
        let plan = select(
            &read.identity.response_id,
            read.identity.custom_llm_provider.as_deref(),
            operation,
            self.facts,
            &self.redactor,
        )?;
        log_call(&mut *log.state, &plan, read.identity, &self.redactor)?;
        let params = read.identity.read_params();
        let spec = ReadSpec {
            extra_headers: read.identity.extra_headers.as_ref(),
            extra_body,
            query,
            timeout: timeout_deadline(read.identity.timeout.as_ref(), self.default_timeout),
            mode: mode_flag(flag),
        };
        let outcome = self.read(&plan, &params, spec, log).await?;
        Ok((outcome, plan))
    }

    // B904 re-encodes the retrieved id from litellm_metadata.model_info.id and the provider
    fn retrieved_reply(
        &self,
        outcome: ReadOutcome,
        plan: &ReadCallPlan,
        identity: &ReadIdentity,
    ) -> Result<TypedReply, Box<LifecycleFailure>> {
        let ReadOutcome::Response(mut reply) = outcome else {
            return Err(Box::new(LifecycleFailure::new(
                LifecycleError::UnexpectedInputItems,
                Some(&plan.provider),
                &self.redactor,
            )));
        };
        self.rewrite(&mut reply.response, Some(&plan.provider), identity)?;
        Ok(*reply)
    }

    // the item list is the provider's own JSON: no id rewrite touches it
    fn item_page(
        &self,
        outcome: ReadOutcome,
        provider: &str,
    ) -> Result<Json, Box<LifecycleFailure>> {
        let ReadOutcome::InputItems(items) = outcome else {
            return Err(Box::new(LifecycleFailure::new(
                LifecycleError::UnexpectedResponse,
                Some(provider),
                &self.redactor,
            )));
        };
        Ok(items)
    }

    // B2416 over the resolved plan; a failed read answers the SDK exception for the selected provider
    async fn read(
        &self,
        plan: &ReadCallPlan,
        params: &ReadParams,
        spec: ReadSpec<'_>,
        log: LifecycleLog<'_, '_>,
    ) -> Result<ReadOutcome, Box<LifecycleFailure>> {
        let request = ReadRequest {
            tag: &plan.tag,
            response_id: &plan.response_id,
            params,
            extra_headers: spec.extra_headers,
            extra_body: spec.extra_body,
            query: spec.query,
            timeout: spec.timeout,
            mode: spec.mode,
            log: LogContext {
                state: log.state,
                env: &log.env,
            },
        };
        self.executor.read(request).await.map_err(|error| {
            Box::new(LifecycleFailure::new(
                LifecycleError::Read(Box::new(error)),
                Some(plan.provider.as_str()),
                &self.redactor,
            ))
        })
    }

    fn rewrite(
        &self,
        response: &mut ResponsesApiResponse,
        provider: Option<&str>,
        identity: &ReadIdentity,
    ) -> Result<(), Box<LifecycleFailure>> {
        let metadata = affinity(identity.litellm_metadata.as_ref())
            .map_err(|error| Box::new(LifecycleFailure::new(error, provider, &self.redactor)))?;
        rewrite_typed_response(response, provider, &metadata).map_err(|error| {
            Box::new(LifecycleFailure::new(
                LifecycleError::Rewrite(error),
                provider,
                &self.redactor,
            ))
        })
    }
}
