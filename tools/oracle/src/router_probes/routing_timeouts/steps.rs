use std::time::{Duration, Instant};

use betterlite_router::routing_timeouts::completion::{
    TimeoutKwargs, TimeoutPhases, TimeoutValue, resolve, supports_httpx_timeout,
};
use betterlite_router::routing_timeouts::global::GlobalRequestTimeout;
use betterlite_router::routing_timeouts::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, RouterTimeouts, TimeoutPlan,
    attempt_timeout, non_stream_timeout, resolve_router_timeouts, stream_timeout,
};
use betterlite_router::routing_timeouts::seconds::Seconds;

use super::wire::{
    CompletionInput, ConfiguredInput, Fields, Payload, RouterInput, TimeoutInput, TimeoutOutput,
};
use crate::router_probes::error::OperationError;

pub(super) fn fields(payload: &Payload) -> Result<Fields, OperationError> {
    let mut fields = Fields::default();
    fill_configured(payload.configured.as_ref(), &mut fields)?;
    fill_completion(payload.completion.as_ref(), &mut fields)?;
    fill_router(payload.router.as_ref(), &mut fields)?;
    Ok(fields)
}

fn fill_configured(
    configured: Option<&ConfiguredInput>,
    output: &mut Fields,
) -> Result<(), OperationError> {
    let Some(input) = configured else {
        return Ok(());
    };
    let global = global_timeout(&input.seconds, input.explicitly_set)?;
    output.configured = global
        .configured()
        .map(|seconds| format!("{:.6}", seconds.0));
    Ok(())
}

fn fill_completion(
    completion: Option<&CompletionInput>,
    output: &mut Fields,
) -> Result<(), OperationError> {
    let Some(input) = completion else {
        return Ok(());
    };
    let global = global_timeout(
        &input.global_timeout.seconds,
        input.global_timeout.explicitly_set,
    )?;
    let kwargs = TimeoutKwargs {
        timeout: optional_seconds("kwargs timeout", input.timeout.as_deref())?,
        request_timeout: optional_seconds(
            "kwargs request_timeout",
            input.request_timeout.as_deref(),
        )?,
    };
    let model_timeout = input
        .model_timeout
        .as_ref()
        .map(timeout_value)
        .transpose()?;
    let phases_supported = supports_httpx_timeout(&input.provider);
    output.resolved = Some(TimeoutOutput::from(resolve(
        model_timeout,
        &kwargs,
        global.configured(),
        phases_supported,
    )));
    output.transport = Some(TimeoutOutput::from(resolve(
        model_timeout,
        &kwargs,
        global.configured(),
        false,
    )));
    Ok(())
}

fn fill_router(router: Option<&RouterInput>, output: &mut Fields) -> Result<(), OperationError> {
    let Some(input) = router else {
        return Ok(());
    };
    let global = global_timeout(
        &input.global_timeout.seconds,
        input.global_timeout.explicitly_set,
    )?;
    let timeouts = resolve_router_timeouts(&router_sources(input)?, global);
    let request = request_timeouts(input)?;
    let deployment = deployment_timeouts(input)?;
    output.stream = stream_timeout(&request, &deployment, &timeouts).map(TimeoutOutput::from);
    output.non_stream =
        non_stream_timeout(&request, &deployment, &timeouts).map(TimeoutOutput::from);
    output.attempt = attempt_timeout(&request, &deployment, &timeouts).map(TimeoutOutput::from);
    fill_plan(input, output, &timeouts, &request, &deployment)
}

fn router_sources(input: &RouterInput) -> Result<RouterTimeoutSources, OperationError> {
    Ok(RouterTimeoutSources {
        timeout: optional_seconds("router_settings.timeout", input.timeout.as_deref())?,
        stream_timeout: optional_seconds(
            "router_settings.stream_timeout",
            input.stream_timeout.as_deref(),
        )?,
        default_timeout: optional_seconds(
            "router_settings.default_litellm_params.timeout",
            input.default_timeout.as_deref(),
        )?,
        default_stream_timeout: optional_seconds(
            "router_settings.default_litellm_params.stream_timeout",
            input.default_stream_timeout.as_deref(),
        )?,
    })
}

fn request_timeouts(input: &RouterInput) -> Result<RequestTimeouts, OperationError> {
    Ok(RequestTimeouts {
        stream: input.kwargs.stream,
        timeout: optional_seconds("kwargs timeout", input.kwargs.timeout.as_deref())?,
        request_timeout: optional_seconds(
            "kwargs request_timeout",
            input.kwargs.request_timeout.as_deref(),
        )?,
        stream_timeout: optional_seconds(
            "kwargs stream_timeout",
            input.kwargs.stream_timeout.as_deref(),
        )?,
    })
}

fn deployment_timeouts(input: &RouterInput) -> Result<DeploymentTimeouts, OperationError> {
    Ok(DeploymentTimeouts {
        timeout: optional_seconds(
            "litellm_params.timeout",
            input.deployment.timeout.as_deref(),
        )?,
        request_timeout: optional_seconds(
            "litellm_params.request_timeout",
            input.deployment.request_timeout.as_deref(),
        )?,
        stream_timeout: optional_seconds(
            "litellm_params.stream_timeout",
            input.deployment.stream_timeout.as_deref(),
        )?,
    })
}

fn fill_plan(
    input: &RouterInput,
    output: &mut Fields,
    router: &RouterTimeouts,
    request: &RequestTimeouts,
    deployment: &DeploymentTimeouts,
) -> Result<(), OperationError> {
    let now = Instant::now();
    let plan = TimeoutPlan::new(now, router, request, deployment).map_err(|error| {
        OperationError::InvalidRequest {
            message: error.to_string(),
        }
    })?;
    let deadline = plan.deadline.checked_duration_since(now).ok_or_else(|| {
        OperationError::InvalidRequest {
            message: "the routing deadline precedes the probe clock".to_owned(),
        }
    })?;
    output.deadline_millis = Some(u64::try_from(deadline.as_millis()).map_err(|_source| {
        OperationError::InvalidRequest {
            message: "the routing deadline does not fit in milliseconds".to_owned(),
        }
    })?);
    if input.cancelled {
        plan.cancellation.cancel();
        output.cancelled = Some(true);
    }
    output.attempt_started = Some(plan.attempt_deadline(now).is_ok());
    fill_expiry(input.elapsed_millis, output, &plan, now)
}

fn fill_expiry(
    elapsed_millis: Option<u64>,
    output: &mut Fields,
    plan: &TimeoutPlan,
    now: Instant,
) -> Result<(), OperationError> {
    let Some(elapsed) = elapsed_millis else {
        return Ok(());
    };
    let fired = now
        .checked_add(Duration::from_millis(elapsed))
        .ok_or_else(|| OperationError::InvalidRequest {
            message: "the elapsed time does not fit in an instant".to_owned(),
        })?;
    let deadline = plan
        .attempt_deadline(now)
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    if fired < deadline {
        return Ok(());
    }
    let error = plan.expired(fired);
    output.expired_status = error.status_code();
    output.expired_code = error.code().map(str::to_owned);
    Ok(())
}

fn global_timeout(
    seconds_text: &str,
    explicitly_set: bool,
) -> Result<GlobalRequestTimeout, OperationError> {
    Ok(GlobalRequestTimeout {
        seconds: seconds("the global request timeout", seconds_text)?,
        explicitly_set,
    })
}

fn timeout_value(input: &TimeoutInput) -> Result<TimeoutValue, OperationError> {
    match input {
        TimeoutInput::Seconds { seconds: text } => {
            Ok(TimeoutValue::Seconds(seconds("model_timeout", text)?))
        }
        TimeoutInput::Phases {
            connect,
            read,
            write,
            pool,
        } => Ok(TimeoutValue::Phases(TimeoutPhases {
            connect: optional_seconds("connect", connect.as_deref())?,
            read: optional_seconds("read", read.as_deref())?,
            write: optional_seconds("write", write.as_deref())?,
            pool: optional_seconds("pool", pool.as_deref())?,
        })),
    }
}

fn optional_seconds(field: &str, text: Option<&str>) -> Result<Option<Seconds>, OperationError> {
    let Some(value) = text else {
        return Ok(None);
    };
    Ok(Some(seconds(field, value)?))
}

fn seconds(field: &str, text: &str) -> Result<Seconds, OperationError> {
    let value: f64 = text
        .parse()
        .map_err(|_source| OperationError::InvalidRequest {
            message: format!("{field} is not a number: {text}"),
        })?;
    Seconds::new(field, value).map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}
