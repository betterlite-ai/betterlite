use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::transformation::EvalsRequestContext;
use betterlite_openai::evals::types::{Eval, UpdateEvalRequest};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use crate::api::eval_create_list::call::EvalsCallParams;
use crate::api::eval_create_list::config::EvalsConfigLookup;
use crate::api::eval_create_list::logging::log_pre_call;
use crate::api::eval_http::exchange::EvalConnection;
use crate::api::passthrough_call::logging::json_object;

use super::call::{EvalRetrieval, EvalUpdate, GetEvalCall, UpdateEvalCall};
use super::error::{EvalsGetUpdateError, EvalsGetUpdateFailureError, failure};
use super::handler::{EvalsGetUpdateHandler, GetEvalHttpCall, UpdateEvalHttpCall};
use super::prepare::{ProviderCall, api_base, prepare};
use super::request::{merge_extra_body, update_request};

// the collaborators the operations call: B9370's handler, the provider configs, the pooled client and B247's logging
pub struct EvalsGetUpdateService<'deps, Handler, Configs> {
    pub handler: &'deps Handler,
    pub configs: &'deps Configs,
    pub client: &'deps ProxiedClient,
    pub core: &'deps CoreSettings,
    pub redactor: &'deps Redactor,
}

// the operation's own input, the update request built before the provider work so a wrong-typed value fails first
enum WireCall<'call> {
    Get(&'call GetEvalCall<'call>),
    Update {
        call: &'call UpdateEvalCall<'call>,
        request: UpdateEvalRequest,
    },
}

// which operation asks, for the ValueError LiteLLM raises when the manager answers None
#[derive(Clone, Copy)]
pub(super) enum Operation {
    Get,
    Update,
}

impl Operation {
    #[must_use]
    pub(super) fn unsupported(self, provider: &str) -> EvalsGetUpdateError {
        match self {
            Self::Get => EvalsGetUpdateError::GetUnsupported {
                provider: provider.to_owned(),
            },
            Self::Update => EvalsGetUpdateError::UpdateUnsupported {
                provider: provider.to_owned(),
            },
        }
    }
}

// which handler call the shared dispatch makes, with the body only the update carries
enum HandlerCall {
    Get,
    Update { body: Object },
}

// the transformed wire pieces and the logging state the shared dispatch hands to the handler
struct Wire<'wire, 'env> {
    url: String,
    headers: Headers,
    optional_params: IndexMap<String, Json>,
    logging: &'wire mut LoggingState,
    env: &'wire PrePostEnv<'env>,
    call: HandlerCall,
}

// LiteLLM spells the blocking and async form of each operation out twice; one async body serves both names
macro_rules! entries {
    (retrieve, $($name:ident),+ $(,)?) => { $(
        pub async fn $name(
            &self,
            retrieval: EvalRetrieval<'_, '_>,
        ) -> Result<Eval, EvalsGetUpdateFailureError> {
            let EvalRetrieval { call, logging, env } = retrieval;
            self.run(WireCall::Get(call), logging, env)
                .await
                .map_err(|error| failure(error, &call.params, self.redactor))
        }
    )+ };
    (update, $($name:ident),+ $(,)?) => { $(
        pub async fn $name(
            &self,
            update: EvalUpdate<'_, '_>,
        ) -> Result<Eval, EvalsGetUpdateFailureError> {
            let EvalUpdate { call, logging, env } = update;
            let request =
                update_request(call).map_err(|error| failure(error, &call.params, self.redactor))?;
            self.run(WireCall::Update { call, request }, logging, env)
                .await
                .map_err(|error| failure(error, &call.params, self.redactor))
        }
    )+ };
}

impl<'deps, Handler, Configs> EvalsGetUpdateService<'deps, Handler, Configs>
where
    Handler: EvalsGetUpdateHandler,
    Configs: EvalsConfigLookup,
{
    entries!(retrieve, get_eval, aget_eval);
    entries!(update, update_eval, aupdate_eval);

    // one body serves both operations: they differ in the transform, the logged params and the handler call
    async fn run(
        &self,
        call: WireCall<'_>,
        logging: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<Eval, EvalsGetUpdateError> {
        let (params, operation) = match call {
            WireCall::Get(get) => (&get.params, Operation::Get),
            WireCall::Update { call: update, .. } => (&update.params, Operation::Update),
        };
        // GenericLiteLLMParams(**kwargs) runs before the provider lookup in LiteLLM, so a wrong-typed api_base fails first
        let base = api_base(params.kwargs)?;
        let prepared = prepare(self.configs, self.core, params, operation)?;
        let context = EvalsRequestContext {
            litellm_params: &prepared.litellm_params,
            headers: &prepared.headers,
        };
        let (url, headers, optional_params, handler_call) =
            transformed(&prepared, call, &base, &context)?;
        self.send(
            &prepared,
            params,
            Wire {
                url,
                headers,
                optional_params,
                logging,
                env,
                call: handler_call,
            },
        )
        .await
    }

    // the one dispatch both operations end in: the pre-call log, then the handler call its variant names
    async fn send(
        &self,
        prepared: &ProviderCall<'deps>,
        params: &EvalsCallParams<'_>,
        wire: Wire<'_, '_>,
    ) -> Result<Eval, EvalsGetUpdateError> {
        log_pre_call(
            &mut *wire.logging,
            params,
            &prepared.provider,
            wire.optional_params,
        )?;
        let connection = EvalConnection {
            config: prepared.config,
            client: self.client,
            url: &wire.url,
            headers: &wire.headers,
        };
        let logging = LogContext {
            state: wire.logging,
            env: wire.env,
        };
        self.deliver(connection, logging, prepared.timeout, wire.call)
            .await
    }

    // the handler call a wire names, with the connection and logging the dispatch already built
    async fn deliver(
        &self,
        connection: EvalConnection<'_>,
        logging: LogContext<'_, '_>,
        timeout: Duration,
        call: HandlerCall,
    ) -> Result<Eval, EvalsGetUpdateError> {
        match call {
            HandlerCall::Get => {
                self.handler
                    .get_eval(GetEvalHttpCall {
                        connection,
                        logging,
                        timeout,
                    })
                    .await
            }
            HandlerCall::Update { body } => {
                self.handler
                    .update_eval(UpdateEvalHttpCall {
                        connection,
                        logging,
                        timeout,
                        body: &body,
                    })
                    .await
            }
        }
        .map_err(EvalsGetUpdateError::from)
    }
}

// the transform and pre-call params of whichever operation asked, with the handler call it makes
fn transformed(
    prepared: &ProviderCall<'_>,
    call: WireCall<'_>,
    base: &str,
    context: &EvalsRequestContext<'_>,
) -> Result<(String, Headers, IndexMap<String, Json>, HandlerCall), EvalsGetUpdateError> {
    match call {
        WireCall::Get(get) => {
            let (url, headers) =
                prepared
                    .config
                    .transform_get_eval_request(get.eval_id, base, context)?;
            let optional_params =
                IndexMap::from([(String::from("eval_id"), Json::Text(get.eval_id.to_owned()))]);
            Ok((url, headers, optional_params, HandlerCall::Get))
        }
        WireCall::Update {
            call: update,
            request,
        } => {
            let (url, headers, mut body) = prepared.config.transform_update_eval_request(
                update.eval_id,
                &request,
                base,
                context,
            )?;
            merge_extra_body(&mut body, update.extra_body)?;
            let optional_params = json_object(&body);
            Ok((url, headers, optional_params, HandlerCall::Update { body }))
        }
    }
}
