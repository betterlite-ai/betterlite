use std::time::{Duration, Instant};

use betterlite_router::routing_timeouts::completion::{
    TimeoutKwargs, TimeoutPhases, TimeoutValue, resolve, supports_httpx_timeout,
};
use betterlite_router::routing_timeouts::global::GlobalRequestTimeout;
use betterlite_router::routing_timeouts::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, RouterTimeouts, TimeoutPlan,
    resolve_router_timeouts, select_timeouts,
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
    let supported = supports_httpx_timeout(&input.provider);
    output.resolved = Some(TimeoutOutput::from(completion_timeout(input, supported)?));
    output.transport = Some(TimeoutOutput::from(completion_timeout(input, false)?));
    Ok(())
}

fn completion_timeout(
    input: &CompletionInput,
    phases_supported: bool,
) -> Result<TimeoutValue, OperationError> {
    let global = global_timeout(
        &input.global_timeout.seconds,
        input.global_timeout.explicitly_set,
    )?;
    let kwargs = TimeoutKwargs {
        timeout: as_seconds(input.timeout.as_deref(), "kwargs timeout")?,
        request_timeout: as_seconds(input.request_timeout.as_deref(), "kwargs request_timeout")?,
    };
    let model_timeout = input
        .model_timeout
        .as_ref()
        .map(timeout_value)
        .transpose()?;
    Ok(resolve(
        model_timeout,
        &kwargs,
        global.configured(),
        phases_supported,
    ))
}

fn fill_router(router: Option<&RouterInput>, output: &mut Fields) -> Result<(), OperationError> {
    let Some(input) = router else {
        return Ok(());
    };
    let global = global_timeout(
        &input.global_timeout.seconds,
        input.global_timeout.explicitly_set,
    )?;
    let probe = probe_timeouts(input)?;
    let timeouts = resolve_router_timeouts(&probe.sources, global);
    let selected = select_timeouts(&probe.request, &probe.deployment, &timeouts);
    output.stream = selected.stream.map(TimeoutOutput::from);
    output.non_stream = Some(TimeoutOutput::from(selected.non_stream));
    output.attempt = Some(TimeoutOutput::from(selected.attempt));
    fill_plan(input, output, &timeouts, &probe.request, &probe.deployment)
}

struct ProbeTimeouts {
    sources: RouterTimeoutSources,
    request: RequestTimeouts,
    deployment: DeploymentTimeouts,
}

fn probe_timeouts(input: &RouterInput) -> Result<ProbeTimeouts, OperationError> {
    let sources = RouterTimeoutSources {
        timeout: as_seconds(input.timeout.as_deref(), "router_settings.timeout")?,
        stream_timeout: as_seconds(
            input.stream_timeout.as_deref(),
            "router_settings.stream_timeout",
        )?,
        default_stream_timeout: as_seconds(
            input.default_stream_timeout.as_deref(),
            "router_settings.default_litellm_params.stream_timeout",
        )?,
    };
    let kwargs = &input.kwargs;
    let request = RequestTimeouts {
        stream: kwargs.stream,
        timeout: as_seconds(kwargs.timeout.as_deref(), "kwargs timeout")?,
        request_timeout: as_seconds(kwargs.request_timeout.as_deref(), "kwargs request_timeout")?,
        stream_timeout: as_seconds(kwargs.stream_timeout.as_deref(), "kwargs stream_timeout")?,
    };
    let params = &input.deployment;
    let deployment = DeploymentTimeouts {
        timeout: as_seconds(params.timeout.as_deref(), "litellm_params.timeout")?,
        request_timeout: as_seconds(
            params.request_timeout.as_deref(),
            "litellm_params.request_timeout",
        )?,
        stream_timeout: as_seconds(
            params.stream_timeout.as_deref(),
            "litellm_params.stream_timeout",
        )?,
    };
    Ok(ProbeTimeouts {
        sources,
        request,
        deployment,
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
    let status = plan.expired(fired).status_code();
    output.expired_status = status;
    output.expired_code = status.map(|code| code.to_string());
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
            connect: as_seconds(connect.as_deref(), "connect")?,
            read: as_seconds(read.as_deref(), "read")?,
            write: as_seconds(write.as_deref(), "write")?,
            pool: as_seconds(pool.as_deref(), "pool")?,
        })),
    }
}

fn as_seconds(text: Option<&str>, field: &str) -> Result<Option<Seconds>, OperationError> {
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
