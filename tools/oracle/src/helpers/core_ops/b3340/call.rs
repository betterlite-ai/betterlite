use betterlite_core::api::video_list_call::{
    VideoListCall, VideoListDeps, VideoListReply, avideo_list, video_list,
};
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_call::config::DEFAULT_TIMEOUT_SECONDS;
use betterlite_core::api::video_remix_call::error::VideoCallFailure;
use betterlite_core::api::video_remix_call::{
    VideoRemixCall, VideoRemixDeps, avideo_remix, video_remix,
};
use betterlite_core::api::video_remix_list_transport::records::Params;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use super::harness::Bench;
use super::project;
use super::{Input, Operation, failed};

pub(crate) struct Outcome {
    pub(crate) response: Value,
    pub(crate) error: Value,
}

pub(crate) fn run(input: &Input, bench: &mut Bench) -> Result<Outcome, Box<HelperError>> {
    match input.operation {
        Operation::Remix => run_remix(input, bench, false),
        Operation::AvideoRemix => run_remix(input, bench, true),
        Operation::List => run_list(input, bench, false),
        Operation::AvideoList => run_list(input, bench, true),
    }
}

fn current_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failed(&error))
}

// The signature's `timeout: Optional[float] = 600`, then the caller's explicit None or number
fn call_timeout(input: &Input) -> Option<f64> {
    match input.timeout.as_ref() {
        None => Some(DEFAULT_TIMEOUT_SECONDS),
        Some(Value::Null) => None,
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_)) => None,
    }
}

fn params(value: Option<&Value>) -> Result<Option<Params>, Box<HelperError>> {
    value
        .map(|value| {
            serde_json::from_value::<Params>(value.clone()).map_err(|error| failed(&error))
        })
        .transpose()
}

fn mock(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    value
        .map(|value| serde_json::from_value::<Json>(value.clone()).map_err(|error| failed(&error)))
        .transpose()
}

fn extra_headers(input: &Input) -> Option<IndexMap<String, String>> {
    if input.extra_headers.is_empty() {
        return None;
    }
    Some(
        input
            .extra_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

fn remix_call(input: &Input) -> Result<VideoRemixCall, Box<HelperError>> {
    Ok(VideoRemixCall {
        video_id: input.video_id.clone().unwrap_or_default(),
        prompt: input.prompt.clone().unwrap_or_default(),
        custom_llm_provider: input.custom_llm_provider.clone(),
        api_key: input.api_key.as_deref().map(SecretString::from),
        api_base: input.api_base.clone(),
        api_version: None,
        extra_headers: extra_headers(input),
        extra_query: params(input.extra_query.as_ref())?,
        extra_body: params(input.extra_body.as_ref())?,
        mock_response: mock(input.mock_response.as_ref())?,
        litellm_metadata: None,
        litellm_call_id: Some(input.litellm_call_id.clone()),
        user: input.user.clone(),
        timeout: call_timeout(input),
        extra: IndexMap::new(),
    })
}

fn list_call(input: &Input) -> Result<VideoListCall, Box<HelperError>> {
    Ok(VideoListCall {
        after: input.after.clone(),
        limit: input.limit,
        order: input.order.clone(),
        custom_llm_provider: input.custom_llm_provider.clone(),
        api_key: input.api_key.as_deref().map(SecretString::from),
        api_base: input.api_base.clone(),
        api_version: None,
        extra_headers: extra_headers(input),
        extra_query: params(input.extra_query.as_ref())?,
        extra_body: params(input.extra_body.as_ref())?,
        mock_response: mock(input.mock_response.as_ref())?,
        litellm_metadata: None,
        litellm_call_id: Some(input.litellm_call_id.clone()),
        user: input.user.clone(),
        timeout: call_timeout(input),
        extra: IndexMap::new(),
    })
}

fn remix_outcome(result: Result<VideoObject, VideoCallFailure>) -> Outcome {
    match result {
        Ok(object) => Outcome {
            response: serde_json::to_value(&object).unwrap_or(Value::Null),
            error: Value::Null,
        },
        Err(failure) => Outcome {
            response: Value::Null,
            error: project::error(&failure.class),
        },
    }
}

fn list_outcome(result: Result<VideoListReply, VideoCallFailure>) -> Outcome {
    match result {
        Ok(reply) => Outcome {
            response: serde_json::to_value(&reply).unwrap_or(Value::Null),
            error: Value::Null,
        },
        Err(failure) => Outcome {
            response: Value::Null,
            error: project::error(&failure.class),
        },
    }
}

fn run_remix(
    input: &Input,
    bench: &mut Bench,
    is_async: bool,
) -> Result<Outcome, Box<HelperError>> {
    let call = remix_call(input)?;
    let pre_post = bench.env.pre_post();
    let deps = VideoRemixDeps {
        configs: &bench.configs,
        logging: &mut bench.state,
        env: &pre_post,
        redactor: bench.env.redactor(),
        request_timeout_seconds: input.request_timeout_seconds.unwrap_or(6000.0),
    };
    if is_async {
        let runner = current_runtime()?;
        let result = runner.block_on(avideo_remix(&bench.transport, deps, &call, None));
        Ok(remix_outcome(result))
    } else {
        let result = video_remix(&bench.transport, deps, &call, None);
        Ok(remix_outcome(result))
    }
}

fn run_list(input: &Input, bench: &mut Bench, is_async: bool) -> Result<Outcome, Box<HelperError>> {
    let call = list_call(input)?;
    let pre_post = bench.env.pre_post();
    let deps = VideoListDeps {
        configs: &bench.configs,
        logging: &mut bench.state,
        env: &pre_post,
        redactor: bench.env.redactor(),
        request_timeout_seconds: input.request_timeout_seconds.unwrap_or(6000.0),
    };
    if is_async {
        let runner = current_runtime()?;
        let resolution = bench.resolver.context();
        let result = runner.block_on(avideo_list(
            &bench.transport,
            deps,
            &resolution,
            &call,
            None,
        ));
        Ok(list_outcome(result))
    } else {
        let result = video_list(&bench.transport, deps, &call, None);
        Ok(list_outcome(result))
    }
}
