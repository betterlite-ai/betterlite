use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::response_resources::response::TypedReply;

use super::call::{
    LifecycleOperation, Resolved, ResponsesLifecycleProvider, ResponsesLifecycleWriteCall, resolve,
};
use super::error::{LifecycleWriteError, ResponsesLifecycleWriteError};
use super::logging::update_logging;
use crate::api::responses_write_http::config::{ResponsesConfig, WriteParams};
use crate::api::responses_write_http::error::WriteError;
use crate::api::responses_write_http::service::{
    CancelCall, ClientSource, DeleteCall, LogContext, WriteCall, WriteDeps, WriteOutcome,
    WriteTarget, prepare, send,
};

// The four SDK entry points over B2417's write transport: the caller supplies the provider config and the client source
pub struct ResponsesLifecycleWrite<'deps, C: ResponsesConfig, S: ClientSource> {
    pub deps: WriteDeps<'deps, C, S>,
    // B198's REQUEST_TIMEOUT, imported rather than owned again
    pub request_timeout: RequestTimeout,
}

// LiteLLM's delete and cancel entries run one body each and differ in the operation and the outcome variant; each async twin decodes first and awaits its sync entry once
macro_rules! lifecycle_entries {
    ($sync:ident, $asynchronous:ident, $outcome:ident, $other:ident, $output:ident) => {
        // the sync entry: decode, refuse, log, then send through the client its own kwargs flag picks
        pub async fn $sync(
            &self,
            call: &ResponsesLifecycleWriteCall,
            state: &mut LoggingState,
            env: &PrePostEnv<'_>,
        ) -> Result<$output, ResponsesLifecycleWriteError> {
            match self.write(call, state, env).await? {
                WriteOutcome::$outcome(answer) => Ok(*answer),
                WriteOutcome::$other(_) => Err(Self::wrong_answer(call)),
            }
        }

        // the async entry: mark its own mode flag true, then await the sync entry's answer once
        pub async fn $asynchronous(
            &self,
            call: &ResponsesLifecycleWriteCall,
            state: &mut LoggingState,
            env: &PrePostEnv<'_>,
        ) -> Result<$output, ResponsesLifecycleWriteError> {
            let marked = call.decoded(&resolve(call));
            self.$sync(&marked, state, env).await
        }
    };
}

impl<C, S> ResponsesLifecycleWrite<'_, C, S>
where
    C: ResponsesConfig,
    S: ClientSource,
{
    lifecycle_entries!(
        delete_responses,
        adelete_responses,
        Deleted,
        Cancelled,
        DeleteResponseResult
    );
    lifecycle_entries!(
        cancel_responses,
        acancel_responses,
        Cancelled,
        Deleted,
        TypedReply
    );

    async fn write(
        &self,
        call: &ResponsesLifecycleWriteCall,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<WriteOutcome, ResponsesLifecycleWriteError> {
        let resolved = resolve(call);
        let provider = validated(call.operation, &resolved)
            .map_err(|error| boundary(error, resolved.provider.as_deref()))?;
        update_logging(state, call, &resolved.response_id, provider).map_err(|error| {
            boundary(
                LifecycleWriteError::Logging(error),
                resolved.provider.as_deref(),
            )
        })?;
        let params = WriteParams {
            api_base: call.params.api_base.clone(),
            api_key: call.params.api_key.clone(),
            ssl_verify: call.params.ssl_verify,
        };
        let write = self.prepared_call(call, &resolved.response_id, &params);
        self.send_write(call, state, env, &write)
            .await
            .map_err(|error| {
                boundary(
                    LifecycleWriteError::Transport(error),
                    resolved.provider.as_deref(),
                )
            })
    }

    async fn send_write(
        &self,
        call: &ResponsesLifecycleWriteCall,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
        write: &WriteCall<'_>,
    ) -> Result<WriteOutcome, WriteError> {
        let prepared = prepare(
            &self.deps,
            call.client_kind(),
            write,
            &mut LogContext { state, env },
        )?;
        send(prepared).await
    }

    fn prepared_call<'call>(
        &self,
        call: &'call ResponsesLifecycleWriteCall,
        response_id: &'call str,
        params: &'call WriteParams,
    ) -> WriteCall<'call> {
        let target = WriteTarget {
            response_id,
            params,
            extra_headers: call.extra_headers.as_ref(),
            // LiteLLM's `timeout or request_timeout`: only a falsy zero falls back, and a negative or non-finite float becomes an already elapsed deadline
            timeout: Some(
                Duration::try_from_secs_f64(resolved_timeout(call.timeout, self.request_timeout))
                    .unwrap_or_default(),
            ),
        };
        match call.operation {
            LifecycleOperation::Delete => WriteCall::Delete(DeleteCall {
                target,
                extra_body: call.extra_body.as_ref(),
            }),
            LifecycleOperation::Cancel => WriteCall::Cancel(CancelCall {
                target,
                extra_body: call.extra_body.as_ref(),
            }),
        }
    }

    fn wrong_answer(call: &ResponsesLifecycleWriteCall) -> ResponsesLifecycleWriteError {
        let resolved = resolve(call);
        boundary(
            LifecycleWriteError::WrongAnswer {
                label: call.operation.label(),
            },
            resolved.provider.as_deref(),
        )
    }
}

fn validated<'call>(
    operation: LifecycleOperation,
    resolved: &'call Resolved<'_>,
) -> Result<&'call str, LifecycleWriteError> {
    let Some(provider) = resolved.provider.as_deref() else {
        return Err(LifecycleWriteError::MissingProvider);
    };
    if ResponsesLifecycleProvider::resolve(provider).is_none() {
        return Err(LifecycleWriteError::Unsupported {
            label: operation.label(),
            provider: provider.to_owned(),
        });
    }
    Ok(provider)
}

// LiteLLM passes `timeout or request_timeout` straight through; zero is the only falsy value that falls back
const fn resolved_timeout(timeout: Option<f64>, request_timeout: RequestTimeout) -> f64 {
    match timeout {
        Some(seconds) if seconds != 0.0_f64 => seconds,
        Some(_) | None => request_timeout.seconds,
    }
}

fn boundary(error: LifecycleWriteError, provider: Option<&str>) -> ResponsesLifecycleWriteError {
    ResponsesLifecycleWriteError {
        error,
        provider: provider.unwrap_or_default().to_owned(),
    }
}
