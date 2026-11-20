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

impl<'deps, C, S> ResponsesLifecycleWrite<'deps, C, S>
where
    C: ResponsesConfig,
    S: ClientSource,
{
    // delete_responses: decode, refuse, log, then send through the client its own kwargs flag picks
    pub async fn delete_responses(
        &self,
        call: &ResponsesLifecycleWriteCall,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<DeleteResponseResult, ResponsesLifecycleWriteError> {
        match self.write(call, state, env).await? {
            WriteOutcome::Deleted(record) => Ok(*record),
            WriteOutcome::Cancelled(_) => Err(self.wrong_answer(call)),
        }
    }

    // adelete_responses: decode first, mark this entry's own flag true, then await the sync entry's answer once
    pub async fn adelete_responses(
        &self,
        call: &ResponsesLifecycleWriteCall,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<DeleteResponseResult, ResponsesLifecycleWriteError> {
        let marked = call.decoded(&resolve(call));
        self.delete_responses(&marked, state, env).await
    }

    // cancel_responses: decode, refuse, log, then send through the client its own kwargs flag picks
    pub async fn cancel_responses(
        &self,
        call: &ResponsesLifecycleWriteCall,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<TypedReply, ResponsesLifecycleWriteError> {
        match self.write(call, state, env).await? {
            WriteOutcome::Cancelled(reply) => Ok(*reply),
            WriteOutcome::Deleted(_) => Err(self.wrong_answer(call)),
        }
    }

    // acancel_responses: decode first, mark this entry's own flag true, then await the sync entry's answer once
    pub async fn acancel_responses(
        &self,
        call: &ResponsesLifecycleWriteCall,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<TypedReply, ResponsesLifecycleWriteError> {
        let marked = call.decoded(&resolve(call));
        self.cancel_responses(&marked, state, env).await
    }

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
        let target = WriteTarget {
            response_id: &resolved.response_id,
            params: &params,
            extra_headers: call.extra_headers.as_ref(),
            // LiteLLM's `timeout or request_timeout`: only a falsy zero falls back, and the value is a float it hands to httpx
            timeout: Some(deadline(resolved_timeout(
                call.timeout,
                self.request_timeout,
            ))),
        };
        let write = match call.operation {
            LifecycleOperation::Delete => WriteCall::Delete(DeleteCall {
                target,
                extra_body: call.extra_body.as_ref(),
            }),
            LifecycleOperation::Cancel => WriteCall::Cancel(CancelCall {
                target,
                extra_body: call.extra_body.as_ref(),
            }),
        };
        let prepared = prepare(
            &self.deps,
            call.client_kind(),
            &write,
            &mut LogContext { state, env },
        )
        .map_err(|error| {
            boundary(
                LifecycleWriteError::Transport(error),
                resolved.provider.as_deref(),
            )
        })?;
        send(prepared).await.map_err(|error| {
            boundary(
                LifecycleWriteError::Transport(error),
                resolved.provider.as_deref(),
            )
        })
    }

    fn wrong_answer(&self, call: &ResponsesLifecycleWriteCall) -> ResponsesLifecycleWriteError {
        let resolved = resolve(call);
        boundary(
            LifecycleWriteError::WrongAnswer {
                operation: call.operation,
            },
            resolved.provider.as_deref(),
        )
    }
}

fn validated(
    operation: LifecycleOperation,
    resolved: &Resolved<'_>,
) -> Result<&str, LifecycleWriteError> {
    let Some(provider) = resolved.provider.as_deref() else {
        return Err(LifecycleWriteError::MissingProvider);
    };
    if ResponsesLifecycleProvider::resolve(provider).is_none() {
        return Err(LifecycleWriteError::Unsupported {
            operation,
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

// Duration cannot hold a negative interval, so a negative or non-finite timeout becomes an already elapsed deadline
fn deadline(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(seconds).unwrap_or_default()
}

fn boundary(error: LifecycleWriteError, provider: Option<&str>) -> ResponsesLifecycleWriteError {
    ResponsesLifecycleWriteError {
        error,
        provider: provider.unwrap_or_default().to_owned(),
    }
}
